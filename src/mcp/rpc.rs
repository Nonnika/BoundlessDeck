//! JSON-RPC as MCP speaks it: the methods, and the answers that need no deck.

use serde_json::{Value, json};

use super::tools::{self, SERVER_NAME};
use super::{PROTOCOL_VERSION, SERVER_VERSION};

/// What only the deck can answer: a tool call or a resource read.
pub enum Job {
    Tool(String, tools::Args),
    Resource(String),
}

/// Runs a job on the deck, on its UI thread.
pub type Runner<'a> = dyn Fn(Job) -> Result<Value, String> + 'a;

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

pub fn tool_failure(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

pub fn tool_result(mut out: Value) -> Value {
    // A picture a tool took (`screenshot`): MCP carries images as their own
    // content, so the base64 leaves the JSON; only its size stays there.
    let mut image = None;
    if let Value::Object(o) = &mut out
        && let Some(Value::Object(pic)) = o.get_mut("image")
        && let Some(data) = pic.remove("data").and_then(|d| d.as_str().map(str::to_string))
    {
        let mime = pic.get("mimeType").and_then(Value::as_str).unwrap_or("image/png").to_string();
        image = Some(json!({"type": "image", "data": data, "mimeType": mime}));
    }
    let text = serde_json::to_string_pretty(&out).unwrap_or_default();
    let mut content = vec![json!({"type": "text", "text": text})];
    content.extend(image);
    let mut res = json!({"content": content, "isError": false});
    match out {
        Value::Object(_) => res["structuredContent"] = out,
        Value::Array(_) => res["structuredContent"] = json!({"blocks": out}),
        _ => {}
    }
    res
}

fn resources() -> Value {
    json!([
        {"uri": "boundlessdeck://deck", "name": "deck", "title": "工作区与画布概况",
         "description": "工作区路径、block 数量、选择集、相机、MCP 状态。", "mimeType": "application/json"},
        {"uri": "boundlessdeck://blocks", "name": "blocks", "title": "全部 block",
         "description": "画布上每个 block 的当前值与可写字段。", "mimeType": "application/json"},
        {"uri": "boundlessdeck://schema", "name": "schema", "title": "block 字段 schema",
         "description": "每种 block 的可写字段、只读字段、默认值与约束。", "mimeType": "application/json"},
    ])
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
    let version = if matches!(asked, "2024-11-05" | "2025-03-26" | "2025-06-18") { asked } else { PROTOCOL_VERSION };
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": {"listChanged": false},
            "resources": {"subscribe": false, "listChanged": false},
            "prompts": {"listChanged": false},
        },
        "serverInfo": {"name": SERVER_NAME, "title": "BoundlessDeck 画布", "version": SERVER_VERSION},
        "instructions": "这是 BoundlessDeck 的无限画布。「block」是画布上的卡片，kind 有 note（笔记：Markdown 文件）、quicklook（QuickLook 便签，会自动消失）、file（本地文件预览）、terminal（终端）、browser（浏览器）、chart（图表）、stack（块堆：用 items 收纳其他 block）、text（文字：按 Markdown 显示，没有卡片）、shape（图形：矩形、椭圆、菱形、三角或箭头，可写字）；文字与图形的颜色、字体、圆角等用 style 设置。先用 deck_info 和 block_schema 了解现场，用 list_blocks / read_selection 读，用 create_block / update_block / move_blocks / delete_blocks 写；排版用 align_blocks（对齐边或中线）、distribute_blocks（等间距）、arrange_blocks（行/列/网格）；想看一眼画布现在的样子就用 screenshot；要带人逐处看画布，用 focus_set 存焦点、focus_play 按顺序放映。工作时必须用 process 报告进度：开始时 preparing、动手时 running（随进展更新 detail/progress）、结束时 done，人会在窗口底部看到它。加、删、移动、堆的收放与排版都进撤销栈，并保存到工作区。",
    })
}

/// The answer to a method that needs no deck, if it is one.
pub fn answer_offline(method: &str, params: &Value) -> Option<Value> {
    Some(match method {
        "initialize" => initialize(params),
        "ping" | "logging/setLevel" => json!({}),
        "tools/list" => json!({"tools": tools::tool_list().into_iter().map(|(name, title, description, schema)| {
            json!({"name": name, "title": title, "description": description, "inputSchema": schema})
        }).collect::<Vec<_>>()}),
        "resources/list" => json!({"resources": resources()}),
        "resources/templates/list" => json!({"resourceTemplates": [{
            "uriTemplate": "boundlessdeck://blocks/{id}", "name": "block", "title": "一个 block",
            "description": "按 block ID 读它的当前值与可写字段。", "mimeType": "application/json",
        }]}),
        "prompts/list" => json!({"prompts": []}),
        _ => return None,
    })
}

/// Answers one message; None for a notification.
pub fn handle(msg: &Value, run: &Runner<'_>) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    if method.starts_with("notifications/") {
        return None;
    }
    let id = msg.get("id").cloned().filter(|v| !v.is_null())?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    if let Some(result) = answer_offline(method, &params) {
        return Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
    let result = match method {
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "tools/call 需要 name"));
            };
            if !tools::tool_names().contains(&name) {
                return Some(error(id, -32602, &format!("没有工具 {name}")));
            }
            let args = match params.get("arguments") {
                None | Some(Value::Null) => tools::Args::new(),
                Some(Value::Object(m)) => m.clone(),
                Some(_) => return Some(json!({"jsonrpc": "2.0", "id": id, "result": tool_failure("参数要是对象")})),
            };
            match run(Job::Tool(name.to_string(), args)) {
                Ok(out) => tool_result(out),
                Err(e) => tool_failure(&e),
            }
        }
        "resources/read" => {
            let Some(uri) = params.get("uri").and_then(Value::as_str).filter(|u| !u.is_empty()) else {
                return Some(error(id, -32602, "resources/read 需要 uri"));
            };
            match run(Job::Resource(uri.to_string())) {
                Ok(out) => json!({"contents": [{
                    "uri": uri, "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&out).unwrap_or_default(),
                }]}),
                Err(e) => return Some(error(id, -32602, &e)),
            }
        }
        _ => return Some(error(id, -32601, &format!("没有方法 {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Answers a body: one message or a batch. None when nothing needs answering.
pub fn handle_body(body: &[u8], run: &Runner<'_>) -> Option<Vec<u8>> {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return Some(serde_json::to_vec(&error(Value::Null, -32700, &format!("JSON 解析失败：{e}"))).unwrap());
        }
    };
    match parsed {
        Value::Array(batch) => {
            let out: Vec<String> =
                batch.iter().filter_map(|m| handle(m, run)).map(|v| serde_json::to_string(&v).unwrap()).collect();
            (!out.is_empty()).then(|| out.join("\n").into_bytes())
        }
        msg => handle(&msg, run).map(|v| serde_json::to_vec(&v).unwrap()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_deck(_: Job) -> Result<Value, String> {
        Err("no deck".into())
    }

    #[test]
    fn answers_the_handshake_and_lists_tools() {
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}), &no_deck).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        let list = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), &no_deck).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), tools::tool_names().len());
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), &no_deck).is_none());
        let call = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "deck_info"}}), &no_deck).unwrap();
        assert_eq!(call["result"]["isError"], true);
        let bad = handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "nope"}), &no_deck).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
    }

    #[test]
    fn a_picture_leaves_the_text_for_image_content() {
        let out = tool_result(json!({
            "camera": {"x": 0.0},
            "image": {"mimeType": "image/png", "width": 2, "height": 1, "data": "AAEC"},
        }));
        let content = out["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1], json!({"type": "image", "data": "AAEC", "mimeType": "image/png"}));
        assert!(!content[0]["text"].as_str().unwrap().contains("AAEC"), "the base64 is not repeated in the text");
        assert_eq!(out["structuredContent"]["image"]["width"], 2);
        assert!(out["structuredContent"]["image"].get("data").is_none());
    }

    #[test]
    fn batches_answer_each_request() {
        let body = br#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#;
        let out = String::from_utf8(handle_body(body, &no_deck).unwrap()).unwrap();
        assert_eq!(out.lines().count(), 2);
        assert!(handle_body(b"{", &no_deck).is_some());
    }
}
