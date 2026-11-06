// a program built on lust adds its own api to lua, the way a game engine adds its drawing:
// here a tiny "engine" with screen.print and ecs.now. its data (the lines on the screen,
// the clock) lives in the lua state as app data, not in the world, and is not journaled;
// the world stays the place of the game's rules.
//   cargo run --example engine
use lust::host::Host;
use lust::mlua::Value;

// what the engine keeps for itself
struct Screen(Vec<String>);
struct Clock(i64);

fn main() -> Result<(), String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    let lua = host.lua();
    lua.set_app_data(Screen(Vec::new()));
    lua.set_app_data(Clock(0));

    // screen.print(text): a table of its own, like love.graphics
    let screen = lua.create_table().map_err(|e| e.to_string())?;
    let print = lua
        .create_function(|lua, text: String| {
            if let Some(mut s) = lua.app_data_mut::<Screen>() {
                s.0.push(text);
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    screen.set("print", print).map_err(|e| e.to_string())?;
    host.add_global("screen", Value::Table(screen))?;

    // ecs.now() -> the tick of the engine: one more function in the table ecs
    let now = lua.create_function(|lua, ()| Ok(lua.app_data_ref::<Clock>().map(|c| c.0).unwrap_or(0))).map_err(|e| e.to_string())?;
    host.add_to_ecs("now", now)?;

    // the game: a type, a role, an operation and a rule, as data in the world
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("system", "kind", "bool"), ("system", "write", "dev"),
            ("x", "kind", "int"), ("x", "write", "dev player"),
            ("role.player", "desc", "moves the hero"),
            ("op.step", "roles", "player"),
            ("op.step", "params", "dx:int"),
            ("op.step", "src", "return function(p) ecs.set('hero', 'x', (ecs.get('hero', 'x') or 0) + p.dx) end"),
            ("sys.draw", "system", "true"),
            ("sys.draw", "src", "return function() screen.print('tick ' .. ecs.now() .. ': hero at ' .. ecs.get('hero', 'x')) end"),
        ] {
            w.set(e, c, v)?;
        }
    }

    // the loop of the engine: each tick the player acts, the rules run and draw
    for (tick, dx) in [(1, "1"), (2, "1"), (3, "-2")] {
        lua.set_app_data(Clock(tick));
        host.run("player", "step", &[("dx", dx)])?;
    }
    for line in &lua.app_data_ref::<Screen>().ok_or("no screen")?.0 {
        println!("{}", line);
    }
    Ok(())
}
