// a kiosk in a few lines: the developer writes the types, a role, an operation, a rule
// and a view as data; the clerk sells; everything lands in the journal, which is saved
// and read back into the same world.
//   cargo run --example kiosk
use lust::ecs::World;
use lust::host::Host;
use lust::store::{self, Store};
use std::path::PathBuf;

const SETUP: &[(&str, &str, &str)] = &[
    // the types of the world are entities too: kind, who may write, what it means
    ("roles", "kind", "names"),
    ("roles", "write", "dev"),
    ("params", "kind", "text"),
    ("params", "write", "dev"),
    ("src", "kind", "code"),
    ("src", "write", "dev"),
    ("system", "kind", "bool"),
    ("system", "write", "dev"),
    ("stock", "kind", "int"),
    ("stock", "write", "dev clerk"),
    ("stock", "desc", "pieces on the shelf"),
    ("alarm", "kind", "bool"),
    ("alarm", "write", "system"),
    // a role is an entity role.<name>
    ("role.clerk", "desc", "sells over the counter"),
    // an operation: who may run it, its parameters and its code (lua, kept in the world)
    ("op.sell", "desc", "sells n pieces of an item"),
    ("op.sell", "roles", "clerk"),
    ("op.sell", "params", "item:id n:int"),
    ("op.sell", "src", "return function(p)
        local left = (ecs.get(p.item, 'stock') or 0) - p.n
        if left < 0 then error('not_enough: ' .. p.item, 0) end
        ecs.set(p.item, 'stock', left)
    end"),
    // a rule (system): runs as the role system after every operation
    ("sys.low", "system", "true"),
    ("sys.low", "src", "return function()
        for _, e in ipairs(ecs.with('stock')) do
            ecs.set(e, 'alarm', ecs.get(e, 'stock') < 3)
        end
    end"),
    // a view: read-only code that gives back a table (json for the program)
    ("view.shelf", "src", "return function()
        local out = {}
        for _, e in ipairs(ecs.with('stock')) do
            out[e] = {stock = ecs.get(e, 'stock'), alarm = ecs.get(e, 'alarm') or false}
        end
        return out
    end"),
    // the goods
    ("tea", "stock", "5"),
    ("coffee", "stock", "2"),
];

fn main() -> Result<(), String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in SETUP {
            w.set(e, c, v)?;
        }
    }

    host.run("clerk", "sell", &[("item", "tea"), ("n", "3")])?;
    println!("sold 3 tea: {}", host.view("shelf", None)?);

    // too many: the operation fails, everything it did is undone, the journal notes why
    let r = host.run("clerk", "sell", &[("item", "coffee"), ("n", "9")]);
    println!("sell 9 coffee: {:?}", r);

    // the clerk may run sell, but may not change its code
    host.world.borrow_mut().current_role = String::from("clerk");
    println!("the clerk rewrites op.sell: {:?}", host.world.borrow_mut().set("op.sell", "src", "return function() end"));

    // the journal: every change, who made it and in which operation, as a hash chain
    let path = std::env::temp_dir().join("lust-kiosk.jsonl");
    let _ = std::fs::remove_file(&path);
    let mut st = Store::new(PathBuf::from(&path), 0);
    st.append(&host.world.borrow().journal)?;
    let (entries, _) = store::load(&path)?.ok_or("no journal")?;
    let n = entries.len();
    let again = World::replay(entries)?;
    println!("{} entries saved to {} and replayed: the same world? {}", n, path.display(), again.head() == host.world.borrow().head());
    for e in host.world.borrow().journal.iter().rev().take(4).collect::<Vec<_>>().into_iter().rev() {
        println!("  #{} {} {} {}.{} = {}", e.seq, e.role, e.op, e.entity, e.comp, e.value);
    }
    Ok(())
}
