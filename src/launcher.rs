// the launcher: what `lust` shows when it is started with a double click and no folder. a
// list of games (the folders with a world.lua) as buttons, in a world of its own that is
// itself a project made of lua files (src/lua/launcher_*.lua) and uses lib.ui. a chosen
// game is handed to the program, which runs it in the same page; F1 in a game comes back.
use crate::project::{build, App};
use crate::screen::Input;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Game {
    pub name: String,
    pub path: PathBuf,
}

// the folders of a folder that hold a world.lua, by name
pub fn find_games(dir: &Path) -> Vec<Game> {
    let mut out: Vec<Game> = match std::fs::read_dir(dir) {
        Ok(list) => list
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir() && e.path().join("world.lua").is_file())
            .map(|e| Game { name: e.file_name().to_string_lossy().to_string(), path: e.path() })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort_by_key(|g| g.name.to_lowercase());
    out
}

// the first of these folders that holds games
pub fn find_games_in(dirs: &[PathBuf]) -> (Vec<Game>, Option<PathBuf>) {
    for d in dirs {
        let g = find_games(d);
        if !g.is_empty() {
            return (g, Some(d.clone()));
        }
    }
    (Vec::new(), None)
}

// a text for lua source: quoted, only plain characters
fn lua_text(s: &str) -> String {
    let clean: String = s.chars().take(200).map(|c| if c.is_alphanumeric() || " -_.,:;!?()/".contains(c) { c } else { '?' }).collect();
    let clean = if clean.trim().is_empty() { String::from("?") } else { clean };
    format!("\"{}\"", clean)
}

pub struct Launcher {
    pub app: App,
    pub games: Vec<Game>,
    picked: Rc<RefCell<Option<usize>>>,
}

impl Launcher {
    // the hint is shown when there are no games
    pub fn new(games: Vec<Game>, hint: &str) -> Result<Launcher, String> {
        let mut world = String::from("return {\n  types = { title = 'text', hint = 'text', sel = 'int' },\n  entities = {\n");
        world.push_str(&format!("    menu = {{ sel = 1, hint = {} }},\n", lua_text(hint)));
        for (i, g) in games.iter().enumerate() {
            world.push_str(&format!("    ['game.{:03}'] = {{ title = {} }},\n", i + 1, lua_text(&g.name.chars().take(40).collect::<String>())));
        }
        world.push_str("  },\n}\n");
        let mut files = std::collections::BTreeMap::new();
        files.insert(String::from("world.lua"), world);
        files.insert(String::from("lib/pick.lua"), String::from(include_str!("lua/launcher_pick.lua")));
        files.insert(String::from("sys/pick.lua"), String::from(include_str!("lua/launcher_sys.lua")));
        files.insert(String::from("draw/pick.lua"), String::from(include_str!("lua/launcher_draw.lua")));
        let (app, _, problems) = build(&files)?;
        if !problems.is_empty() {
            return Err(format!("the launcher: {}", problems.join("; ")));
        }
        let picked = Rc::new(RefCell::new(None));
        let p = Rc::clone(&picked);
        let f = app
            .host
            .lua()
            .create_function(move |_, name: String| {
                let n = name.strip_prefix("game.").and_then(|d| d.parse::<usize>().ok()).unwrap_or(0);
                *p.borrow_mut() = if n >= 1 { Some(n - 1) } else { None };
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        app.host.add_to_ecs("launch", f)?;
        Ok(Launcher { app, games, picked })
    }

    // one frame; an error of the launcher itself is shown as a frame, not as a crash
    pub fn frame(&mut self, dt: f64, input: Input) -> String {
        let (t, d) = self.app.frame(dt, input);
        match (t, d) {
            (Ok(()), Ok(f)) => f,
            (Err(e), _) | (_, Err(e)) => format!("size 800 600\nclear #300\ntext 20 20 18 #fff launcher: {}\n", e),
        }
    }

    // what the launcher told the display this tick (see screen::Exchange::events)
    pub fn take_events(&mut self) -> Vec<String> {
        self.app.screen.take_events()
    }

    // the game the person chose, once
    pub fn take_pick(&mut self) -> Option<&Game> {
        let n = self.picked.borrow_mut().take()?;
        self.games.get(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn games_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("lust-games-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for n in ["snake", "Pong", "empty", "tetris_2"] {
            fs::create_dir_all(d.join(n)).unwrap();
            if n != "empty" {
                fs::write(d.join(n).join("world.lua"), "return {}").unwrap();
            }
        }
        d
    }

    #[test]
    fn games_are_the_folders_with_a_world() {
        let d = games_dir();
        let names: Vec<String> = find_games(&d).into_iter().map(|g| g.name).collect();
        assert_eq!(names, vec!["Pong", "snake", "tetris_2"]);
        let (g, at) = find_games_in(&[d.join("nothing"), d.clone()]);
        assert_eq!(g.len(), 3);
        assert_eq!(at, Some(d.clone()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_launcher_lists_games_and_a_click_or_enter_picks_one() {
        let d = games_dir();
        let mut l = Launcher::new(find_games(&d), "none").unwrap();
        let f = l.frame(0.016, Input::default());
        for t in ["Pong", "snake", "tetris_2"] {
            assert!(f.contains(&format!("#ffffff {}", t)), "{}", f);
        }
        // down, down, enter: the third
        for key in ["down", "down", "enter"] {
            let mut i = Input::default();
            i.pressed.insert(key.to_string());
            l.frame(0.016, i);
        }
        assert_eq!(l.take_pick().map(|g| g.name.clone()), Some(String::from("tetris_2")));
        assert!(l.take_pick().is_none());
        // a click on the first button (centered at 400, from y 150; 28 pixels text, 14 pad)
        let mut i = Input { mouse: (400.0, 160.0, true), ..Default::default() };
        i.pressed.insert(String::from("mouse"));
        l.frame(0.016, i);
        assert_eq!(l.take_pick().map(|g| g.name.clone()), Some(String::from("Pong")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn many_games_make_pages_and_no_games_say_what_to_do() {
        let d = std::env::temp_dir().join(format!("lust-many-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for i in 0..11 {
            fs::create_dir_all(d.join(format!("g{:02}", i))).unwrap();
            fs::write(d.join(format!("g{:02}", i)).join("world.lua"), "return {}").unwrap();
        }
        let mut l = Launcher::new(find_games(&d), "none").unwrap();
        let first = l.frame(0.016, Input::default());
        assert!(first.contains("page 1 of 2") && first.contains("g07") && !first.contains("g08"), "{}", first);
        // down past the end of the page goes to the next page
        for _ in 0..8 {
            let mut i = Input::default();
            i.pressed.insert(String::from("down"));
            l.frame(0.016, i);
        }
        let second = l.frame(0.016, Input::default());
        assert!(second.contains("page 2 of 2") && second.contains("g08") && second.contains("g10") && !second.contains("g07"), "{}", second);
        let mut e = Launcher::new(Vec::new(), "No games found. Put game folders next to lust, in a folder named games.").unwrap();
        let f = e.frame(0.016, Input::default());
        assert!(f.contains("No games found."), "{}", f);
        let _ = fs::remove_dir_all(&d);
    }
}
