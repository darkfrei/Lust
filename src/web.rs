// a display in the browser: a page on 127.0.0.1 shows the frames of a world (screen.rs) on
// a canvas and sends the keys and the mouse back. std only, a thread per connection, never
// lua: the loop of the program and these threads share an Exchange behind a mutex. this is
// one display of several possible ones: another module (a window) may take the same
// Exchange instead. leave it out with the feature "web".
//   GET /        the page
//   POST /frame  the input as text (screen::Input::parse) in, the last frame out; its first
//                line is "status <why the world stopped>" (empty while it runs)
// only requests whose Host is 127.0.0.1 or localhost are answered, so a web page elsewhere
// cannot reach the world through the browser (dns rebinding)
use crate::screen::Exchange;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const HEAD_MAX: usize = 8 * 1024;
const BODY_MAX: usize = 4 * 1024;

pub const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>lust</title>
<style>html,body{margin:0;height:100%;background:#111;overflow:hidden}canvas{display:block;margin:auto}#s{position:fixed;left:8px;bottom:8px;right:8px;color:#f77;font:14px monospace;white-space:pre-wrap}</style>
</head><body><canvas id="c"></canvas><div id="s"></div><script>
var c = document.getElementById('c'), g = c.getContext('2d'), st = document.getElementById('s');
var down = new Set(), hits = new Set(), mx = 0, my = 0, mb = 0, W = 0, H = 0, k = 1;
var names = {ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', ' ': 'space', Enter: 'enter', Escape: 'escape', Shift: 'shift', Control: 'ctrl', Alt: 'alt', Tab: 'tab', Backspace: 'backspace'};
function key(e) { return names[e.key] || e.key.toLowerCase(); }
function keyDown(e) { var n = key(e); if (!down.has(n)) hits.add(n); down.add(n); if (names[e.key]) e.preventDefault(); }
function keyUp(e) { down.delete(key(e)); }
function pos(e) { var r = c.getBoundingClientRect(); mx = (e.clientX - r.left) / k; my = (e.clientY - r.top) / k; }
function mouseDown(e) { pos(e); mb = 1; hits.add('mouse'); }
function mouseUp() { mb = 0; }
function fit() { k = Math.min(innerWidth / W, innerHeight / H); c.width = Math.round(W * k); c.height = Math.round(H * k); }
addEventListener('keydown', keyDown); addEventListener('keyup', keyUp); addEventListener('blur', function () { down.clear(); });
c.addEventListener('mousemove', pos); c.addEventListener('mousedown', mouseDown); addEventListener('mouseup', mouseUp);
addEventListener('resize', function () { if (W) fit(); });
function draw(t) {
  var lines = t.split('\n'), status = '';
  for (var i = 0; i < lines.length; i++) {
    var l = lines[i]; if (!l) continue;
    var p = l.split(' '), o = p[0];
    if (o === 'status') { status = l.slice(7); }
    else if (o === 'size') { var w = +p[1], h = +p[2]; if (w !== W || h !== H) { W = w; H = h; fit(); } g.setTransform(k, 0, 0, k, 0, 0); g.clearRect(0, 0, W, H); }
    else if (o === 'clear') { g.fillStyle = p[1]; g.fillRect(0, 0, W, H); }
    else if (o === 'rect') { g.fillStyle = p[5]; g.fillRect(+p[1], +p[2], +p[3], +p[4]); }
    else if (o === 'circle') { g.fillStyle = p[4]; g.beginPath(); g.arc(+p[1], +p[2], +p[3], 0, 2 * Math.PI); g.fill(); }
    else if (o === 'line') { g.strokeStyle = p[6]; g.lineWidth = +p[5]; g.beginPath(); g.moveTo(+p[1], +p[2]); g.lineTo(+p[3], +p[4]); g.stroke(); }
    else if (o === 'text') { g.fillStyle = p[4]; g.font = (+p[3]) + 'px monospace'; g.textBaseline = 'top'; g.fillText(p.slice(5).join(' '), +p[1], +p[2]); }
  }
  st.textContent = status;
}
function again() { requestAnimationFrame(loop); }
function lost() { st.textContent = 'lust is not running'; setTimeout(loop, 500); }
function loop() {
  var body = 'down=' + Array.from(down).join(',') + ';hit=' + Array.from(hits).join(',') + ';mouse=' + mx.toFixed(1) + ',' + my.toFixed(1) + ',' + mb;
  hits.clear();
  fetch('/frame', {method: 'POST', body: body}).then(function (r) { return r.text(); }).then(function (t) { draw(t); again(); }, lost);
}
loop();
</script></body></html>
"#;

fn respond(s: &mut TcpStream, code: &str, kind: &str, body: &str) -> std::io::Result<()> {
    let head = format!("HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n", code, kind, body.len());
    s.write_all(head.as_bytes())?;
    s.write_all(body.as_bytes())?;
    s.flush()
}

fn host_ok(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{}", port) || host == format!("localhost:{}", port)
}

// one connection: requests one after another (keep-alive) until it closes
fn serve_conn(stream: TcpStream, ex: Arc<Mutex<Exchange>>, port: u16) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut out = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
        let (mut length, mut host, mut size, mut close) = (0usize, String::new(), line.len(), false);
        loop {
            let mut h = String::new();
            if r.read_line(&mut h)? == 0 {
                return Ok(());
            }
            size += h.len();
            if size > HEAD_MAX {
                return respond(&mut out, "431 Request Header Fields Too Large", "text/plain", "");
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap_or(BODY_MAX + 1),
                    "host" => host = v.trim().to_string(),
                    "connection" => close = v.trim().eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }
        if length > BODY_MAX {
            return respond(&mut out, "413 Payload Too Large", "text/plain", "");
        }
        let mut body = vec![0u8; length];
        r.read_exact(&mut body)?;
        if !host_ok(&host, port) {
            respond(&mut out, "403 Forbidden", "text/plain", "only 127.0.0.1")?;
            return Ok(());
        }
        match (method.as_str(), path.as_str()) {
            ("GET", "/") => respond(&mut out, "200 OK", "text/html; charset=utf-8", PAGE)?,
            ("POST", "/frame") => {
                let text = {
                    let mut e = ex.lock().unwrap_or_else(|p| p.into_inner());
                    e.put_input(&String::from_utf8_lossy(&body));
                    format!("status {}\n{}", e.status.replace('\n', " "), e.frame)
                };
                respond(&mut out, "200 OK", "text/plain; charset=utf-8", &text)?;
            }
            _ => respond(&mut out, "404 Not Found", "text/plain", "")?,
        }
        if close {
            return Ok(());
        }
    }
}

// starts the display: the address it listens on (127.0.0.1 only; port 0 takes a free one)
pub fn serve(port: u16, ex: Arc<Mutex<Exchange>>) -> Result<SocketAddr, String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{}: {}", port, e))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let port = addr.port();
    thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let ex = Arc::clone(&ex);
            thread::spawn(move || {
                let _ = serve_conn(s, ex, port);
            });
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(addr: SocketAddr, req: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn the_page_the_frame_and_the_input() {
        let ex = Arc::new(Mutex::new(Exchange::default()));
        ex.lock().unwrap().put_frame(String::from("size 10 10\nrect 1 1 2 2 #fff\n"), "");
        let addr = serve(0, Arc::clone(&ex)).unwrap();
        let host = format!("127.0.0.1:{}", addr.port());
        let page = ask(addr, &format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", host));
        assert!(page.starts_with("HTTP/1.1 200 OK") && page.contains("<canvas"));
        let body = "down=up;hit=space;mouse=1,2,0";
        let f = ask(addr, &format!("POST /frame HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", host, body.len(), body));
        assert!(f.ends_with("status \nsize 10 10\nrect 1 1 2 2 #fff\n"), "{}", f);
        let i = ex.lock().unwrap().take_input();
        assert!(i.down.contains("up") && i.pressed.contains("space"));
        // a request for another host (a page elsewhere, dns rebinding) is refused
        let bad = ask(addr, "GET / HTTP/1.1\r\nHost: evil.example:80\r\nConnection: close\r\n\r\n");
        assert!(bad.starts_with("HTTP/1.1 403"));
    }
}
