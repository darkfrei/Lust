// mcp: a door for an AI into a lust world, over the Model Context Protocol.
// three parts, each usable alone:
//   1. the protocol: JSON-RPC 2.0 messages in and out (handle, handle_text). the program
//      chooses the transport (HTTP, stdio) and the tools (the trait Tools)
//   2. a queue of proposals: an AI proposes an operation of its role, it is checked and
//      waits; only the program (a person) confirms it, and then it runs as that role.
//      there is no tool that confirms
//   3. three ready tools over a world and a role: lust_ops (what the role may do),
//      lust_propose and lust_status (WorldTools)
// this file is optional: build without the feature "mcp", or delete the file and its line
// in lib.rs, and the rest of lust does not change

use crate::ecs::valid_text;
use crate::host::{clip, Host};
use serde_json::{json, Value};
use std::collections::BTreeMap;

// the protocol versions this door speaks, the newest first
pub const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

// ---------- 1. the protocol ----------

// a tool as the AI sees it in tools/list
pub struct Tool {
    pub name: String,
    pub description: String,
    // a JSON schema of the arguments, an object
    pub input: Value,
    // the tool only reads
    pub read_only: bool,
}

// what a program offers through the door
pub trait Tools {
    // the name and version of the server, and a short instruction for the AI
    fn server(&self) -> (String, String, String);
    fn tools(&self) -> Vec<Tool>;
    // the text of the answer; Err is a tool error the AI can read and fix
    fn call(&mut self, name: &str, args: &Value) -> Result<String, String>;
}

fn rpc_error(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn list_json(tools: &[Tool]) -> Value {
    let list: Vec<Value> = tools
        .iter()
        .map(|t| {
            let mut v = json!({"name": t.name, "description": t.description, "inputSchema": t.input});
            if t.read_only {
                v["annotations"] = json!({"readOnlyHint": true});
            }
            v
        })
        .collect();
    json!({ "tools": list })
}

// one JSON-RPC message in, one out; None for notifications and responses (over HTTP: 202)
pub fn handle<T: Tools + ?Sized>(t: &mut T, msg: &Value) -> Option<Value> {
    if !msg.is_object() {
        return Some(rpc_error(Value::Null, -32600, "batches are not supported"));
    }
    let id = msg.get("id").cloned();
    let method = match msg.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            // a response from the client, or garbage
            return if id.is_some() && msg.get("result").is_none() && msg.get("error").is_none() {
                Some(rpc_error(id.unwrap_or(Value::Null), -32600, "no method"))
            } else {
                None
            };
        }
    };
    let id = match id {
        Some(i) if i.is_string() || i.is_number() => i,
        Some(_) => return Some(rpc_error(Value::Null, -32600, "bad id")),
        None => return None,
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
            let version = if SUPPORTED.contains(&asked) { asked } else { SUPPORTED[0] };
            let (name, ver, instructions) = t.server();
            Some(rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": name, "version": ver},
                    "instructions": instructions
                }),
            ))
        }
        "ping" => Some(rpc_result(id, json!({}))),
        "tools/list" => Some(rpc_result(id, list_json(&t.tools()))),
        "tools/call" => {
            let name = match params.get("name").and_then(|v| v.as_str()) {
                Some(n) => n.to_string(),
                None => return Some(rpc_error(id, -32602, "missing tool name")),
            };
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !args.is_object() {
                return Some(rpc_error(id, -32602, "arguments must be an object"));
            }
            // an unknown tool is a protocol error; a bad request to a known tool is a tool
            // error the model can read and fix
            if !t.tools().iter().any(|x| x.name == name) {
                return Some(rpc_error(id, -32602, "unknown tool"));
            }
            Some(rpc_result(
                id,
                match t.call(&name, &args) {
                    Ok(text) => tool_text(text, false),
                    Err(e) => tool_text(e, true),
                },
            ))
        }
        _ => Some(rpc_error(id, -32601, "method not found")),
    }
}

// the same for one line of text (stdio: one JSON message per line); a line that is not
// JSON gets the parse error of JSON-RPC
pub fn handle_text<T: Tools + ?Sized>(t: &mut T, line: &str) -> Option<String> {
    match serde_json::from_str::<Value>(line) {
        Ok(v) => handle(t, &v).map(|r| r.to_string()),
        Err(_) => Some(rpc_error(Value::Null, -32700, "parse error").to_string()),
    }
}

// ---------- 2. the queue of proposals ----------

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Pending,
    Confirmed,
    Rejected,
    Failed(String),
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Confirmed => "confirmed",
            Status::Rejected => "rejected",
            Status::Failed(_) => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub role: String,
    pub op: String,
    pub args: BTreeMap<String, String>,
    pub why: String,
    pub status: Status,
}

pub struct Proposals {
    pub list: Vec<Proposal>,
    next: u64,
    // at most this many wait at once
    pub max_pending: usize,
    // of the decided ones, the newest this many are kept
    pub max_decided: usize,
    // at most this many bytes in one request (operation, arguments and why together)
    pub max_request: usize,
}

const ARGS_MAX: usize = 32;

impl Default for Proposals {
    fn default() -> Self {
        Proposals::new()
    }
}

impl Proposals {
    pub fn new() -> Proposals {
        Proposals { list: Vec::new(), next: 1, max_pending: 20, max_decided: 50, max_request: 64 * 1024 }
    }

    pub fn get(&self, id: u64) -> Option<&Proposal> {
        self.list.iter().find(|p| p.id == id)
    }

    pub fn pending(&self) -> impl Iterator<Item = &Proposal> {
        self.list.iter().filter(|p| p.status == Status::Pending)
    }

    // the AI side: the request is checked as the role would run it (the operation, the
    // right to run it, every argument) and queued. nothing runs here
    pub fn propose(&mut self, host: &Host, role: &str, op: &str, args: BTreeMap<String, String>, why: &str) -> Result<u64, String> {
        if args.len() > ARGS_MAX {
            return Err(String::from("too_many_args"));
        }
        if !why.is_empty() && !valid_text(why) {
            return Err(String::from("bad_text: why"));
        }
        if self.pending().count() >= self.max_pending {
            return Err(String::from("too_many_pending"));
        }
        let size = op.len() + why.len() + args.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>();
        if size > self.max_request {
            return Err(format!("too_long: request {} bytes, at most {}", size, self.max_request));
        }
        let pairs: Vec<(&str, &str)> = args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        host.check(role, op, &pairs)?;
        let id = self.next;
        self.next += 1;
        self.list.push(Proposal { id, role: role.to_string(), op: op.to_string(), args, why: why.to_string(), status: Status::Pending });
        self.trim();
        Ok(id)
    }

    fn trim(&mut self) {
        let decided = self.list.iter().filter(|p| p.status != Status::Pending).count();
        let mut drop = decided.saturating_sub(self.max_decided);
        self.list.retain(|p| {
            if drop > 0 && p.status != Status::Pending {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }

    fn pending_index(&self, id: u64) -> Result<usize, String> {
        match self.list.iter().position(|p| p.id == id) {
            None => Err(String::from("no_such_proposal")),
            Some(i) if self.list[i].status != Status::Pending => Err(String::from("already_decided")),
            Some(i) => Ok(i),
        }
    }

    // the person's side, the only road from a proposal to the world: it runs as the role
    // that proposed it, never as more. the journal marks it "mcp <id>"
    pub fn confirm(&mut self, host: &Host, id: u64) -> Result<Status, String> {
        let i = self.pending_index(id)?;
        let p = self.list[i].clone();
        let pairs: Vec<(&str, &str)> = p.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let status = match host.run_from(&format!("mcp {}", id), &p.role, &p.op, &pairs) {
            Ok(()) => Status::Confirmed,
            Err(reason) => Status::Failed(reason),
        };
        self.list[i].status = status.clone();
        Ok(status)
    }

    pub fn reject(&mut self, id: u64) -> Result<(), String> {
        let i = self.pending_index(id)?;
        self.list[i].status = Status::Rejected;
        Ok(())
    }
}

// ---------- 3. ready tools over a world and a role ----------

// what an AI may do in the world as one role: see the operations of the role, propose one,
// ask what became of it. it sees no values: what it should know is the program's choice
// (a tool of its own, see Tools)
pub struct WorldTools<'a> {
    pub host: &'a Host,
    pub role: &'a str,
    pub proposals: &'a mut Proposals,
}

impl Tools for WorldTools<'_> {
    fn server(&self) -> (String, String, String) {
        (
            String::from("lust"),
            String::from(env!("CARGO_PKG_VERSION")),
            String::from("Read lust_ops first. lust_propose only queues a request; a person confirms it, then it runs as your role."),
        )
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: String::from("lust_ops"),
                description: String::from("The operations your role may propose, each with its typed parameters (? marks an optional one) and what it does."),
                input: json!({"type": "object", "properties": {}}),
                read_only: true,
            },
            Tool {
                name: String::from("lust_propose"),
                description: String::from("Propose one operation. It is checked and queued; nothing changes until a person confirms it. All argument values are strings."),
                input: json!({
                    "type": "object",
                    "properties": {
                        "op": {"type": "string"},
                        "args": {"type": "object", "additionalProperties": {"type": "string"}},
                        "why": {"type": "string", "description": "one sentence for the person"}
                    },
                    "required": ["op", "args"]
                }),
                read_only: false,
            },
            Tool {
                name: String::from("lust_status"),
                description: String::from("What became of a proposal: pending, confirmed, rejected or failed."),
                input: json!({"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}),
                read_only: true,
            },
        ]
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "lust_ops" => Ok(ops_text(self.host, self.role)),
            "lust_propose" => propose_from(self.proposals, self.host, self.role, args).map(|id| format!("queued as {}; waiting for the person", id)),
            "lust_status" => status_text(self.proposals, args),
            _ => Err(String::from("unknown_tool")),
        }
    }
}

// the operations of a role as lines: op name(param:kind, param:kind?): what it does
pub fn ops_text(host: &Host, role: &str) -> String {
    let mut out = format!("role {}\n", role);
    for o in host.ops_of(role) {
        let ps: Vec<String> = o.params.iter().map(|p| format!("{}:{}{}", p.name, p.kind, if p.required { "" } else { "?" })).collect();
        out.push_str(&format!("op {}({}): {}\n", o.name, ps.join(", "), o.desc));
    }
    out
}

// the arguments of a propose tool ({op, args, why}) into the queue
pub fn propose_from(q: &mut Proposals, host: &Host, role: &str, args: &Value) -> Result<u64, String> {
    let op = args.get("op").and_then(|v| v.as_str()).ok_or_else(|| String::from("missing_param: op"))?;
    let obj = args.get("args").and_then(|v| v.as_object()).ok_or_else(|| String::from("missing_param: args"))?;
    let mut map = BTreeMap::new();
    for (k, v) in obj {
        let s = v.as_str().ok_or_else(|| format!("bad_text: {} must be a string", clip(k, 40)))?;
        map.insert(k.clone(), s.to_string());
    }
    let why = args.get("why").and_then(|v| v.as_str()).unwrap_or("");
    q.propose(host, role, op, map, why)
}

// the answer of a status tool ({id})
pub fn status_text(q: &Proposals, args: &Value) -> Result<String, String> {
    let id = args.get("id").and_then(|v| v.as_u64()).ok_or_else(|| String::from("missing_param: id"))?;
    match q.get(id) {
        None => Err(String::from("no_such_proposal")),
        Some(p) => Ok(match &p.status {
            Status::Failed(r) => format!("failed: {}", r),
            other => other.as_str().to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a world with one role (clerk) and one operation (bump)
    fn host() -> Host {
        let h = Host::new().unwrap();
        {
            let mut w = h.world.borrow_mut();
            w.current_role = String::from("dev");
            for (e, c, v) in [
                ("role.clerk", "desc", "a clerk"),
                ("roles", "kind", "names"),
                ("roles", "write", "dev"),
                ("params", "kind", "text"),
                ("params", "write", "dev"),
                ("src", "kind", "code"),
                ("src", "write", "dev"),
                ("count", "kind", "int"),
                ("count", "write", "clerk"),
                ("op.bump", "desc", "adds n to the counter"),
                ("op.bump", "params", "n:int"),
                ("op.bump", "roles", "clerk"),
                ("op.bump", "src", "return function(p) ecs.set('c', 'count', (ecs.get('c', 'count') or 0) + p.n) end"),
            ] {
                w.set(e, c, v).unwrap();
            }
        }
        h
    }

    fn call(t: &mut WorldTools, name: &str, args: Value) -> Value {
        let m = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
        handle(t, &m).unwrap()["result"].clone()
    }

    #[test]
    fn the_protocol_answers_and_lists_the_tools() {
        let h = host();
        let mut q = Proposals::new();
        let mut t = WorldTools { host: &h, role: "clerk", proposals: &mut q };
        let r = handle(&mut t, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "lust");
        let r = handle(&mut t, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 3);
        assert!(handle(&mut t, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let r = handle(&mut t, &json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).unwrap();
        assert_eq!(r["error"]["code"], -32601);
        assert!(handle_text(&mut t, "{nope").unwrap().contains("-32700"));
    }

    #[test]
    fn the_ai_sees_its_operations_and_only_proposes() {
        let h = host();
        let mut q = Proposals::new();
        {
            let mut t = WorldTools { host: &h, role: "clerk", proposals: &mut q };
            let r = call(&mut t, "lust_ops", json!({}));
            assert!(r["content"][0]["text"].as_str().unwrap().contains("op bump(n:int): adds n to the counter"));
            let r = call(&mut t, "lust_propose", json!({"op": "bump", "args": {"n": "5"}, "why": "more"}));
            assert_eq!(r["isError"], false, "{}", r);
            assert_eq!(call(&mut t, "lust_status", json!({"id": 1}))["content"][0]["text"], "pending");
        }
        // nothing ran yet
        assert!(h.world.borrow().value("c", "count").is_none());
        assert_eq!(q.confirm(&h, 1).unwrap(), Status::Confirmed);
        assert_eq!(h.world.borrow().value("c", "count"), Some("5"));
        let mut t = WorldTools { host: &h, role: "clerk", proposals: &mut q };
        assert_eq!(call(&mut t, "lust_status", json!({"id": 1}))["content"][0]["text"], "confirmed");
    }

    #[test]
    fn bad_proposals_are_tool_errors_and_nothing_confirms() {
        let h = host();
        let mut q = Proposals::new();
        let mut t = WorldTools { host: &h, role: "clerk", proposals: &mut q };
        assert_eq!(call(&mut t, "lust_propose", json!({"op": "bump", "args": {"n": "x"}}))["isError"], true);
        assert_eq!(call(&mut t, "lust_propose", json!({"op": "format_disk", "args": {}}))["isError"], true);
        assert_eq!(call(&mut t, "lust_propose", json!({"op": "bump", "args": {"n": 5}}))["isError"], true);
        let r = handle(&mut t, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "lust_confirm", "arguments": {"id": 1}}})).unwrap();
        assert_eq!(r["error"]["code"], -32602);
        assert_eq!(q.list.len(), 0);
    }

    #[test]
    fn a_proposal_runs_as_its_role_and_is_decided_once() {
        let h = host();
        let mut q = Proposals::new();
        let id = q.propose(&h, "clerk", "bump", BTreeMap::from([(String::from("n"), String::from("2"))]), "").unwrap();
        q.reject(id).unwrap();
        assert_eq!(q.confirm(&h, id), Err(String::from("already_decided")));
        assert!(h.world.borrow().value("c", "count").is_none());
        q.max_pending = 1;
        q.propose(&h, "clerk", "bump", BTreeMap::from([(String::from("n"), String::from("1"))]), "").unwrap();
        let r = q.propose(&h, "clerk", "bump", BTreeMap::from([(String::from("n"), String::from("1"))]), "");
        assert_eq!(r, Err(String::from("too_many_pending")));
        let last = h.world.borrow().journal.len();
        assert_eq!(q.confirm(&h, 2).unwrap(), Status::Confirmed);
        let w = h.world.borrow();
        assert!(w.journal[last..].iter().any(|e| e.value.contains("from mcp 2")), "the journal names where it came from");
        assert!(w.journal[last..].iter().any(|e| e.role == "clerk" && e.comp == "count"));
    }
}
