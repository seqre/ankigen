//! End-to-end: drive the language server over an in-memory connection, the
//! way an editor would over stdio.

use std::path::Path;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::Url;
use serde_json::{Value, json};

struct Client {
    conn: Connection,
    server: Option<std::thread::JoinHandle<()>>,
    next_id: i32,
}

impl Client {
    /// Start a server and complete the initialize handshake.
    fn start() -> Client {
        let (server_conn, conn) = Connection::memory();
        let server = std::thread::spawn(move || ankigen::lsp::serve(&server_conn).unwrap());
        let mut client = Client {
            conn,
            server: Some(server),
            next_id: 0,
        };
        let init = client.request(
            "initialize",
            json!({ "capabilities": { "general": { "positionEncodings": ["utf-8", "utf-16"] } } }),
        );
        assert_eq!(init["capabilities"]["positionEncoding"], "utf-8");
        client.notify("initialized", json!({}));
        client
    }

    fn notify(&self, method: &str, params: Value) {
        let n = Notification::new(method.into(), params);
        self.conn.sender.send(Message::Notification(n)).unwrap();
    }

    /// Send a request and return its result, skipping notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let req = Request::new(id.clone(), method.into(), params);
        self.conn.sender.send(Message::Request(req)).unwrap();
        loop {
            if let Message::Response(Response {
                id: got,
                response_result,
            }) = self.recv()
                && got == id
            {
                return response_result.unwrap_or_else(|e| panic!("{method} failed: {e:?}"));
            }
        }
    }

    fn recv(&self) -> Message {
        self.conn
            .receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("server went quiet")
    }

    /// The next `publishDiagnostics` for `uri`, as `(line, message)` pairs.
    fn diagnostics(&self, uri: &Url) -> Vec<(u64, String)> {
        loop {
            if let Message::Notification(n) = self.recv()
                && n.method == "textDocument/publishDiagnostics"
                && n.params["uri"] == uri.as_str()
            {
                return n.params["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| {
                        (
                            d["range"]["start"]["line"].as_u64().unwrap(),
                            d["message"].as_str().unwrap().to_string(),
                        )
                    })
                    .collect();
            }
        }
    }

    /// The next `n` `publishDiagnostics`, as `(uri, diagnostic count)` sorted
    /// by uri: the server publishes open documents in no fixed order.
    fn diagnostic_counts(&self, n: usize) -> Vec<(Url, usize)> {
        let mut counts = Vec::new();
        while counts.len() < n {
            if let Message::Notification(n) = self.recv()
                && n.method == "textDocument/publishDiagnostics"
            {
                let uri = Url::parse(n.params["uri"].as_str().unwrap()).unwrap();
                counts.push((uri, n.params["diagnostics"].as_array().unwrap().len()));
            }
        }
        counts.sort();
        counts
    }

    fn open(&self, path: &Path, text: &str) -> Url {
        let uri = Url::from_file_path(path).unwrap();
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "ankigen", "version": 1, "text": text } }),
        );
        uri
    }

    fn change(&self, uri: &Url, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }],
            }),
        );
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        self.server.take().unwrap().join().unwrap();
    }
}

#[test]
fn diagnostics_follow_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let client = Client::start();
    let uri = client.open(&tmp.path().join("a.cards"), "q: Q\n\nq: Q2\na: A\nt: T\n");

    let diags = client.diagnostics(&uri);
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0].0, 0);
    assert!(
        diags[0]
            .1
            .starts_with("card has `q:` but no answer\nadd `a:`"),
        "message carries the help: {:?}",
        diags[0].1
    );
    assert_eq!(
        diags[1],
        (4, "use either `a:` or `t:`, not both".to_string())
    );

    client.change(&uri, 2, "q: Q\na: A\n");
    assert!(client.diagnostics(&uri).is_empty());
}

#[test]
fn duplicate_ids_are_found_across_the_deck_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let card = format!("// @id {id} model=type-in\nq: Q\nt: 1\n");
    // A built deck tree: `deck-id.txt` in the file's directory and its parent.
    let sub = tmp.path().join("deck/sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(tmp.path().join("deck/deck-id.txt"), "").unwrap();
    std::fs::write(sub.join("deck-id.txt"), "").unwrap();
    std::fs::write(tmp.path().join("deck/other.cards"), &card).unwrap();

    let client = Client::start();
    let a = client.open(&sub.join("a.cards"), &card);
    let diags = client.diagnostics(&a);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].0, 0);
    assert!(diags[0].1.contains("duplicate @id") && diags[0].1.contains("other.cards"));

    // Opening the other file republishes both: each now blames the other.
    let other = client.open(&tmp.path().join("deck/other.cards"), &card);
    assert_eq!(
        client.diagnostic_counts(2),
        [(other.clone(), 1), (a.clone(), 1)]
    );

    // An unsaved edit to the other file clears the error in both.
    client.change(&other, 2, "q: Q\nt: 1\n");
    assert_eq!(client.diagnostic_counts(2), [(other, 0), (a, 0)]);
}

#[test]
fn semantic_tokens_are_delta_encoded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut client = Client::start();
    let uri = client.open(&tmp.path().join("a.cards"), "q: é **b**\nt: 200\n");
    client.diagnostics(&uri);

    let tokens = client.request(
        "textDocument/semanticTokens/full",
        json!({ "textDocument": { "uri": uri } }),
    );
    // (Δline, Δstart, length, type, modifiers); columns are UTF-8 bytes here.
    // Types index the legend: 1 keyword, 9 string, 12 strong.
    assert_eq!(
        tokens["data"],
        json!([0, 0, 2, 1, 0, 0, 6, 5, 12, 0, 1, 0, 2, 1, 0, 0, 3, 3, 9, 0])
    );
}

#[test]
fn hover_and_completion() {
    let tmp = tempfile::tempdir().unwrap();
    let mut client = Client::start();
    let uri = client.open(&tmp.path().join("a.cards"), "q: hello\nr: hola\n\nto");
    client.diagnostics(&uri);

    let at = |line: u32, character: u32| json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } });
    let hover = client.request("textDocument/hover", at(1, 4));
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .starts_with("**Basic + Reversed**")
    );
    assert_eq!(hover["range"]["end"], json!({ "line": 1, "character": 7 }));
    assert_eq!(client.request("textDocument/hover", at(3, 1)), Value::Null);

    let items = client.request("textDocument/completion", at(3, 2));
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["label"], "topic:");
    assert_eq!(
        items[0]["textEdit"],
        json!({
            "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 2 } },
            "newText": "topic: ",
        })
    );
}

#[test]
fn missing_media_is_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let client = Client::start();
    let uri = client.open(
        &tmp.path().join("a.cards"),
        "q: no answer\n\nq: Q\na: [image:gone.png]\n",
    );
    let severities: Vec<u64> = loop {
        if let Message::Notification(n) = client.recv()
            && n.method == "textDocument/publishDiagnostics"
            && n.params["uri"] == uri.as_str()
        {
            let diags = n.params["diagnostics"].as_array().unwrap();
            break diags
                .iter()
                .map(|d| d["severity"].as_u64().unwrap())
                .collect();
        }
    };
    // LSP severities: 1 = error, 2 = warning.
    assert_eq!(severities, [1, 2]);
}
