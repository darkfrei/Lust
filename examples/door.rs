// the door for an AI (lust::mcp) in one sitting: the messages an MCP client would send,
// handled in process. the AI sees what its role may do and proposes; nothing changes
// until the person confirms. the program chooses the transport: here none, in radost
// HTTP, for a local client stdio (one JSON message per line, mcp::handle_text)
//   cargo run --example door
use lust::host::Host;
use lust::mcp::{handle, Proposals, WorldTools};
use serde_json::{json, Value};

fn send(host: &Host, q: &mut Proposals, msg: Value) -> Value {
    let mut door = WorldTools { host, role: "clerk", proposals: q };
    let answer = handle(&mut door, &msg).unwrap_or(Value::Null);
    println!("AI -> {}\n   <- {}\n", msg, answer);
    answer
}

fn main() -> Result<(), String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("stock", "kind", "int"), ("stock", "write", "dev clerk"),
            ("role.clerk", "desc", "sells over the counter"),
            ("op.sell", "desc", "sells n pieces of an item"),
            ("op.sell", "roles", "clerk"),
            ("op.sell", "params", "item:id n:int"),
            ("op.sell", "src", "return function(p) ecs.set(p.item, 'stock', ecs.get(p.item, 'stock') - p.n) end"),
            ("tea", "stock", "5"),
        ] {
            w.set(e, c, v)?;
        }
    }
    let mut q = Proposals::new();
    let call = |name: &str, args: Value| json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});

    send(&host, &mut q, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}));
    send(&host, &mut q, call("lust_ops", json!({})));
    send(&host, &mut q, call("lust_propose", json!({"op": "sell", "args": {"item": "tea", "n": "2"}, "why": "a customer wants two"})));
    println!("tea before the person decides: {}", host.world.borrow().value("tea", "stock").unwrap_or("-"));

    // the person sees the proposal (in a window, a console, a message) and confirms it
    q.confirm(&host, 1)?;
    println!("tea after the person confirmed: {}\n", host.world.borrow().value("tea", "stock").unwrap_or("-"));
    send(&host, &mut q, call("lust_status", json!({"id": 1})));
    // there is no tool that confirms
    send(&host, &mut q, call("lust_confirm", json!({"id": 1})));
    Ok(())
}
