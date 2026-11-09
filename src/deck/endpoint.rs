//! The deck's MCP endpoint: the socket agents reach it on, and the jobs
//! they send, run on the UI thread against the model.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext, Context};

use super::{DeckView, MemoRt};

impl DeckView {
    /// Serves this deck to agents: its socket, its descriptor, the active-deck registry.
    pub fn start_mcp(&mut self, cx: &mut Context<Self>) {
        use crate::mcp::{self, rpc::Job};
        type Reply = std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>;
        let (tx, rx) = smol::channel::unbounded::<(Job, mcp::Caller, Reply)>();
        // Each job runs on the UI thread, in the deck's own add/remove/undo path.
        cx.spawn(async move |this, cx| {
            while let Ok((job, caller, reply)) = rx.recv().await {
                let out = this
                    .update(cx, |d, cx| d.run_job(job, &caller, cx))
                    .unwrap_or_else(|_| Err("这个工作区已经关了".into()));
                let _ = reply.send(out);
            }
        })
        .detach();
        let run: Arc<mcp::server::DeckRunner> = Arc::new(move |job, caller| {
            let (reply, answer) = std::sync::mpsc::sync_channel(1);
            tx.send_blocking((job, caller.clone(), reply)).map_err(|_| "这个工作区已经关了".to_string())?;
            answer.recv_timeout(Duration::from_secs(15)).map_err(|_| "窗口 15s 没有响应".to_string())?
        });
        let root = self.m.root.clone();
        let sock = mcp::socket_path(&root);
        match mcp::server::Server::start(&sock, root.to_string_lossy().into_owned(), run) {
            Ok(server) => {
                let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or("boundlessdeck".into());
                let names = mcp::tools::tool_names();
                let ep = mcp::Endpoint {
                    name: mcp::tools::SERVER_NAME.into(),
                    transport: "unix".into(),
                    socket: server.socket.to_string_lossy().into_owned(),
                    token: server.token.clone(),
                    bridge: mcp::BridgeCmd {
                        command: exe,
                        args: vec!["mcp-stdio".into(), "--workspace".into(), root.to_string_lossy().into_owned()],
                    },
                    tools: names.len(),
                    tool_names: names.into_iter().map(String::from).collect(),
                    protocol_version: mcp::PROTOCOL_VERSION.into(),
                };
                if let Err(e) = mcp::publish(&root, &ep) {
                    self.mcp_err = Some(format!("写 MCP 配置：{e}"));
                }
                self.bridge = Some(ep.bridge.clone());
                self.mcp = Some(server);
                if self.project_mcp
                    && let Err(e) = self.set_project_mcp(true, cx)
                {
                    self.mcp_err = Some(e);
                }
            }
            Err(e) => self.mcp_err = Some(e.to_string()),
        }
    }

    /// Writes the project-level configs now (on), or stops writing them (off).
    pub fn set_project_mcp(&mut self, on: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.project_mcp = on;
        cx.notify();
        if !on {
            return Ok(());
        }
        let Some(b) = &self.bridge else { return Ok(()) };
        let warnings = crate::mcp::config::write_project_configs(&self.m.root, &b.command, &b.args);
        if warnings.is_empty() { Ok(()) } else { Err(warnings.join("；")) }
    }

    pub(super) fn stop_mcp(&mut self) {
        if let Some(s) = self.mcp.take() {
            s.stop();
            crate::mcp::unpublish(&self.m.root);
        }
    }

    /// The window as it shows, for the `screenshot` tool.
    fn photograph(&self, cx: &mut Context<Self>) -> Result<crate::screenshot::Shot, String> {
        let Some(win) = self.win else {
            return Err("这个窗口还没有画出来".into());
        };
        cx.update_window(win, |_, window, _| crate::screenshot::capture(window))
            .map_err(|e| format!("这个窗口已经关了：{e}"))?
    }

    pub(super) fn run_job(
        &mut self,
        job: crate::mcp::rpc::Job,
        caller: &crate::mcp::Caller,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        use crate::mcp::{rpc::Job, tools::Ctx};
        let here = crate::mcp::tools::Here {
            caller: caller.clone(),
            focused: self.focus_target(),
        };
        let info = match &self.mcp {
            Some(s) => serde_json::json!({
                "running": true, "socket": s.socket.to_string_lossy(), "startedAt": s.started.to_rfc3339(),
                "tools": crate::mcp::tools::tool_names().len(), "configFile": ".boundlessdeck/mcp.json",
            }),
            None => serde_json::json!({"running": false, "error": self.mcp_err}),
        };
        // Mid-glide, tools see the camera where it is going.
        let shown = self.glide.map(|g| std::mem::replace(&mut self.m.cam, g.to));
        // The picture is taken from the window as it shows, before the call changes anything.
        let shot = match &job {
            Job::Tool(name, _) if name == "screenshot" => Some(self.photograph(cx)),
            _ => None,
        };
        let mut ctx = Ctx { m: &mut self.m, view: self.view, mcp: info, here, dirty: false, memos: vec![], reload: vec![], view_from: None, shot, process: None };
        let out = match job {
            Job::Tool(name, args) => ctx.call(&name, &args),
            Job::Resource(uri) => ctx.read_resource(&uri),
        };
        let (dirty, memos, reload, view_from, process) = (ctx.dirty, ctx.memos, ctx.reload, ctx.view_from, ctx.process);
        if let Some(r) = process {
            self.report_task(r, cx);
        }
        for (id, text) in memos {
            let rt = self.rts.entry(id).or_default();
            if rt.memo.loaded {
                rt.memo.text = text.clone();
                rt.memo.saved = text;
                rt.memo.modified = None;
            }
        }
        for id in reload {
            if let Some(rt) = self.rts.get_mut(&id) {
                rt.lines = None;
                rt.doc = None;
                rt.word = None;
                rt.pdf = None;
                rt.pdf_img = None;
                rt.md_cache = None;
                rt.face_md_cache = None;
                rt.chart_cache = None;
                rt.memo = MemoRt::default();
                // The open tab's page goes again where it was sent; the other tabs keep theirs.
                match self.m.block(&id).and_then(|b| b.tabs.get(b.tab)) {
                    Some(tab) => {
                        rt.webs.remove(&tab.key);
                    }
                    None => rt.webs.clear(),
                }
                rt.web_err = None;
            }
        }
        if dirty {
            self.after_edit(cx);
        }
        // Saved where it goes (above), then shown going there from where it shows now.
        match (view_from, shown) {
            (Some((from, animate)), shown) => {
                let to = self.m.cam;
                self.glide_to(shown.unwrap_or(from), to, animate, cx);
            }
            (None, Some(cam)) => self.m.cam = cam,
            (None, None) => {}
        }
        out
    }
}
