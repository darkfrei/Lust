// the smallest use of lust: one type, one role, one operation, one journal
//   cargo run --example hello
use lust::host::Host;

fn main() -> Result<(), String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("hits", "kind", "int"), ("hits", "write", "dev guest"),
            ("role.guest", "desc", "may only knock"),
            ("op.knock", "roles", "guest"),
            ("op.knock", "src", "return function() ecs.set('door', 'hits', (ecs.get('door', 'hits') or 0) + 1) end"),
        ] {
            w.set(e, c, v)?;
        }
    }
    host.run("guest", "knock", &[])?;
    host.run("guest", "knock", &[])?;
    println!("hits: {}", host.world.borrow().value("door", "hits").unwrap_or("0"));
    for e in host.world.borrow().journal.iter().rev().take(2).rev() {
        println!("#{} {} {} {}.{} = {}", e.seq, e.role, e.op, e.entity, e.comp, e.value);
    }
    Ok(())
}
