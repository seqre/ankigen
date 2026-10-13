//! `ankigen lsp`: a language server for `.cards` files, over stdio.
//!
//! It runs the same parser and checks as `ankigen build`, so an editor shows
//! exactly what a build would report. The server only reads: minting ids and
//! writing them back stays with `build`.
//!
//! Layout: [`analysis`] derives everything from one document's text (no
//! protocol types), [`text`] converts byte offsets to LSP positions, and this
//! module is the protocol loop plus the cross-file part (duplicate `@id`s).

pub mod analysis;
pub mod text;
pub mod tokens;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Request as _, SemanticTokensFullRequest};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, InitializeParams, InitializeResult, PositionEncodingKind,
    PublishDiagnosticsParams, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, Url,
};
use walkdir::WalkDir;

use crate::error::{AnkigenError, Result};
use crate::model::Diag;
use crate::pipeline::has_card_ext;
use crate::source::cards::parse_lenient;

use analysis::{Analysis, analyze, duplicate_id_diags};
use text::{Encoding, LineIndex};
use tokens::{TokenKind, semantic_tokens};

type ServerResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Serve LSP on stdin/stdout until the client shuts the server down.
pub fn run() -> Result<()> {
    let (connection, io_threads) = Connection::stdio();
    let fail = |e: String| AnkigenError::Io(format!("language server: {e}"));
    serve(&connection).map_err(|e| fail(e.to_string()))?;
    drop(connection);
    io_threads.join().map_err(|e| fail(e.to_string()))
}

/// Run the protocol over `connection` (stdio in production, an in-memory pair
/// in tests) until shutdown.
pub fn serve(connection: &Connection) -> ServerResult<()> {
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)?;

    // UTF-8 columns are free for us (the parser works in bytes); fall back to
    // the protocol's mandatory UTF-16 when the client does not offer them.
    let offers_utf8 = params
        .capabilities
        .general
        .and_then(|g| g.position_encodings)
        .is_some_and(|encs| encs.contains(&PositionEncodingKind::UTF8));
    let (encoding, encoding_kind) = if offers_utf8 {
        (Encoding::Utf8, PositionEncodingKind::UTF8)
    } else {
        (Encoding::Utf16, PositionEncodingKind::UTF16)
    };

    let result = InitializeResult {
        capabilities: ServerCapabilities {
            position_encoding: Some(encoding_kind),
            semantic_tokens_provider: Some(
                lsp_types::SemanticTokensServerCapabilities::SemanticTokensOptions(
                    lsp_types::SemanticTokensOptions {
                        legend: lsp_types::SemanticTokensLegend {
                            token_types: TokenKind::ALL
                                .iter()
                                .map(|k| lsp_types::SemanticTokenType::new(k.name()))
                                .collect(),
                            token_modifiers: Vec::new(),
                        },
                        full: Some(lsp_types::SemanticTokensFullOptions::Bool(true)),
                        ..Default::default()
                    },
                ),
            ),
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::FULL),
                    save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                    ..Default::default()
                },
            )),
            ..Default::default()
        },
        server_info: Some(ServerInfo {
            name: "ankigen".into(),
            version: Some(env!("CARGO_PKG_VERSION").into()),
        }),
    };
    connection.initialize_finish(id, serde_json::to_value(result)?)?;

    let mut server = Server {
        connection,
        encoding,
        docs: HashMap::new(),
        disk_ids: HashMap::new(),
    };
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                server.on_request(req)?;
            }
            Message::Notification(n) => server.on_notification(n)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// An open document, as last sent by the client.
struct Doc {
    path: PathBuf,
    /// The directory tree whose card files share this document's id space.
    scope: PathBuf,
    version: i32,
    text: String,
    index: LineIndex,
    analysis: Analysis,
}

impl Doc {
    fn new(path: PathBuf, version: i32, text: String) -> Doc {
        Doc {
            scope: id_scope(&path),
            index: LineIndex::new(&text),
            analysis: analyze(&text, &path),
            path,
            version,
            text,
        }
    }
}

struct Server<'a> {
    connection: &'a Connection,
    encoding: Encoding,
    docs: HashMap<Url, Doc>,
    /// The `@id`s of every card file on disk under an open document's scope.
    /// Refreshed on open/save/close; open documents override their entry.
    disk_ids: HashMap<PathBuf, Vec<String>>,
}

impl Server<'_> {
    fn on_request(&mut self, req: Request) -> ServerResult<()> {
        let result = match req.method.as_str() {
            SemanticTokensFullRequest::METHOD => {
                let p: lsp_types::SemanticTokensParams = serde_json::from_value(req.params)?;
                serde_json::to_value(self.semantic_tokens(&p.text_document.uri))?
            }
            // Say so rather than leave the client waiting.
            _ => {
                let resp = Response::new_err(
                    req.id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("unsupported request: {}", req.method),
                );
                self.connection.sender.send(Message::Response(resp))?;
                return Ok(());
            }
        };
        let resp = Response::new_ok(req.id, result);
        self.connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    /// Encode the document's tokens the LSP way: each as five integers,
    /// positioned relative to the one before.
    fn semantic_tokens(&self, uri: &Url) -> Option<lsp_types::SemanticTokens> {
        let doc = self.docs.get(uri)?;
        let mut data = Vec::new();
        let mut prev = lsp_types::Position::new(0, 0);
        for token in semantic_tokens(&doc.text, &doc.analysis.file) {
            let range = doc.index.range(&doc.text, token.range, self.encoding);
            let same_line = range.start.line == prev.line;
            data.push(lsp_types::SemanticToken {
                delta_line: range.start.line - prev.line,
                delta_start: range.start.character - if same_line { prev.character } else { 0 },
                length: range.end.character - range.start.character,
                token_type: token.kind.index(),
                token_modifiers_bitset: 0,
            });
            prev = range.start;
        }
        Some(lsp_types::SemanticTokens {
            result_id: None,
            data,
        })
    }

    fn on_notification(&mut self, n: Notification) -> ServerResult<()> {
        match n.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let p: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(n.params)?;
                let doc = p.text_document;
                let opened = Doc::new(uri_path(&doc.uri), doc.version, doc.text);
                self.scan_disk(&opened.scope.clone());
                self.docs.insert(doc.uri, opened);
                self.publish_all()?;
            }
            DidChangeTextDocument::METHOD => {
                let p: lsp_types::DidChangeTextDocumentParams = serde_json::from_value(n.params)?;
                // Sync is FULL, so the last change carries the whole text.
                if let Some(change) = p.content_changes.into_iter().next_back()
                    && let Some(doc) = self.docs.get_mut(&p.text_document.uri)
                {
                    *doc = Doc::new(doc.path.clone(), p.text_document.version, change.text);
                    self.publish_all()?;
                }
            }
            DidSaveTextDocument::METHOD => {
                let p: lsp_types::DidSaveTextDocumentParams = serde_json::from_value(n.params)?;
                if let Some(scope) = self.docs.get(&p.text_document.uri).map(|d| d.scope.clone()) {
                    self.scan_disk(&scope);
                    self.publish_all()?;
                }
            }
            DidCloseTextDocument::METHOD => {
                let p: lsp_types::DidCloseTextDocumentParams = serde_json::from_value(n.params)?;
                if let Some(doc) = self.docs.remove(&p.text_document.uri) {
                    self.publish(p.text_document.uri, None, Vec::new())?;
                    // The file counts from disk again.
                    self.scan_disk(&doc.scope);
                    self.publish_all()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Re-read the ids of every card file under `scope`.
    fn scan_disk(&mut self, scope: &Path) {
        self.disk_ids.retain(|path, _| !path.starts_with(scope));
        for entry in WalkDir::new(scope).into_iter().flatten() {
            let path = entry.path();
            if !entry.file_type().is_file() || !has_card_ext(path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let ids = parse_lenient(&text, path)
                .file
                .cards
                .into_iter()
                .filter_map(|c| c.spec.id)
                .collect();
            self.disk_ids.insert(path.to_path_buf(), ids);
        }
    }

    /// The ids used by every other card file in `doc`'s scope, taking open
    /// documents from memory and the rest from disk.
    fn other_ids(&self, doc: &Doc) -> HashMap<String, PathBuf> {
        let mut ids = HashMap::new();
        let open: Vec<&Doc> = self.docs.values().collect();
        for (path, file_ids) in &self.disk_ids {
            if path.starts_with(&doc.scope) && !open.iter().any(|d| &d.path == path) {
                ids.extend(file_ids.iter().map(|id| (id.clone(), path.clone())));
            }
        }
        for other in open {
            if other.path != doc.path && other.path.starts_with(&doc.scope) {
                let cards = &other.analysis.file.cards;
                ids.extend(
                    cards
                        .iter()
                        .filter_map(|c| c.spec.id.clone())
                        .map(|id| (id, other.path.clone())),
                );
            }
        }
        ids
    }

    /// Publish diagnostics for every open document: an edit to one can add or
    /// clear a duplicate-id error in another.
    fn publish_all(&self) -> ServerResult<()> {
        for (uri, doc) in &self.docs {
            let duplicates = duplicate_id_diags(&doc.analysis.file, &self.other_ids(doc));
            let diagnostics = doc
                .analysis
                .diags
                .iter()
                .chain(&duplicates)
                .map(|d| self.diagnostic(doc, d))
                .collect();
            self.publish(uri.clone(), Some(doc.version), diagnostics)?;
        }
        Ok(())
    }

    fn diagnostic(&self, doc: &Doc, d: &Diag) -> Diagnostic {
        let span = d.span.byte_start..d.span.byte_start + d.span.byte_len;
        let message = match &d.help {
            Some(help) => format!("{}\n{help}", d.message),
            None => d.message.clone(),
        };
        Diagnostic {
            range: doc.index.range(&doc.text, span, self.encoding),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("ankigen".into()),
            message,
            ..Default::default()
        }
    }

    fn publish(
        &self,
        uri: Url,
        version: Option<i32>,
        diagnostics: Vec<Diagnostic>,
    ) -> ServerResult<()> {
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        };
        let n = Notification::new(PublishDiagnostics::METHOD.into(), params);
        self.connection.sender.send(Message::Notification(n))?;
        Ok(())
    }
}

/// The filesystem path of a document (its URI path verbatim for the rare
/// non-`file:` scheme, where nothing on disk will match anyway).
fn uri_path(uri: &Url) -> PathBuf {
    uri.to_file_path()
        .unwrap_or_else(|_| PathBuf::from(uri.path()))
}

/// The directory tree searched for duplicate ids: the document's directory,
/// widened to the topmost ancestor of an unbroken chain of directories that
/// each hold a `deck-id.txt` (i.e. the deck tree a build has already seen).
fn id_scope(path: &Path) -> PathBuf {
    let mut scope = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    if scope.join("deck-id.txt").is_file() {
        while let Some(parent) = scope.parent()
            && parent.join("deck-id.txt").is_file()
        {
            scope = parent.to_path_buf();
        }
    }
    scope
}
