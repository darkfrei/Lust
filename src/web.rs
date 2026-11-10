// a display in the browser: a page on 127.0.0.1 shows the frames of a world (screen.rs) on
// a canvas and sends the keys and the mouse back. std only, a thread per connection, never
// lua: the loop of the program and these threads share an Exchange behind a mutex. this is
// one display of several possible ones: another module (a window) may take the same
// Exchange instead. leave it out with the feature "web".
//   GET /        the page
//   POST /frame  the input as text (screen::Input::parse) in, the last frame out; its first
//                line is "status <why the world stopped>" (empty while it runs), then
//                "events <n>" (the number of the last event) and an "event <n> <text>"
//                for each event after the ack the page sent (screen::Exchange::events)
//   POST /ping   a hidden page is still there     POST /bye   the page was closed
// only requests whose Host is 127.0.0.1 or localhost are answered, so a web page elsewhere
// cannot reach the world through the browser (dns rebinding)
use crate::screen::{self, Exchange};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const HEAD_MAX: usize = 8 * 1024;
const BODY_MAX: usize = 4 * 1024;

pub const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>lust</title>
<style>html,body{margin:0;height:100%;background:#111;overflow:hidden}canvas{display:block;margin:auto}#m{position:fixed;right:8px;top:8px;display:none;padding:4px 10px;border-radius:12px;background:rgba(0,0,0,.65);color:#ddd;font:14px monospace}#m i{display:inline-block;width:10px;height:10px;border-radius:5px;background:#f33;margin-right:8px;vertical-align:middle}#m b{display:inline-block;height:6px;width:0;background:#6f6;margin-left:8px;vertical-align:middle;border-radius:3px}#s{position:fixed;left:8px;bottom:8px;right:8px;color:#f77;font:14px monospace;white-space:pre-wrap}</style>
</head><body><canvas id="c"></canvas><div id="m"></div><div id="s"></div><script>
var c = document.getElementById('c'), g = c.getContext('2d'), st = document.getElementById('s');
var down = new Set(), hits = new Set(), mx = 0, my = 0, mb = 0, W = 0, H = 0, k = 1;
var names = {ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', ' ': 'space', Enter: 'enter', Escape: 'escape', Shift: 'shift', Control: 'ctrl', Alt: 'alt', Tab: 'tab', Backspace: 'backspace', F1: 'f1'};
var FONT = 'Consolas, "DejaVu Sans Mono", "Liberation Mono", "Courier New", monospace', LINE = 1.25;
g.font = '100px ' + FONT;
var cw = g.measureText('M').width / 100;
// the microphone: only when the world asks for it (gfx.mic). the browser asks the person and
// shows its own recording sign; only the loudness (a number 0..1) is sent, never the sound
var mbox = document.getElementById('m'), mState = 'off', mLevel = 0, mCtx = null, mStream = null, mAn = null, mData = null, mWant = 0;
function mShow() {
  if (mState === 'off') { mbox.style.display = 'none'; return; }
  mbox.style.display = 'block';
  if (mState === 'on') mbox.innerHTML = '<i></i>microphone on<b id="ml"></b>';
  else if (mState === 'asking') mbox.innerHTML = 'microphone: allow it in the browser';
  else mbox.innerHTML = 'microphone: blocked in the browser';
}
function mStart() {
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) { mState = 'denied'; mShow(); return; }
  mState = 'asking'; mShow();
  navigator.mediaDevices.getUserMedia({audio: true}).then(function (s) {
    mStream = s; mCtx = new (window.AudioContext || window.webkitAudioContext)();
    mAn = mCtx.createAnalyser(); mAn.fftSize = 1024; mCtx.createMediaStreamSource(s).connect(mAn);
    mData = new Uint8Array(mAn.fftSize); mState = 'on'; mShow();
  }, function () { mState = 'denied'; mShow(); });
}
function mStop() {
  if (mStream) mStream.getTracks().forEach(function (t) { t.stop(); });
  if (mCtx) mCtx.close();
  mStream = null; mCtx = null; mAn = null; mState = 'off'; mLevel = 0; mShow();
}
function mRead() {
  if (mState !== 'on') return 0;
  if (mCtx.state === 'suspended') mCtx.resume();
  mAn.getByteTimeDomainData(mData);
  var sum = 0;
  for (var i = 0; i < mData.length; i++) { var v = (mData[i] - 128) / 128; sum += v * v; }
  mLevel = Math.max(Math.min(1, Math.sqrt(sum / mData.length) * 5), mLevel * 0.9);
  var bar = document.getElementById('ml'); if (bar) bar.style.width = Math.round(mLevel * 60) + 'px';
  return mLevel;
}
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
  var lines = t.split('\n'), status = '', want = false, last = -1, fresh = [];
  for (var i = 0; i < lines.length; i++) {
    var l = lines[i]; if (!l) continue;
    var p = l.split(' '), o = p[0];
    if (o === 'status') { status = l.slice(7); }
    else if (o === 'mic') { want = true; }
    else if (o === 'events') { last = +p[1]; }
    else if (o === 'event') { fresh.push([+p[1], p.slice(2)]); }
    else if (o === 'size') { var w = +p[1], h = +p[2]; if (w !== W || h !== H) { W = w; H = h; fit(); } g.setTransform(k, 0, 0, k, 0, 0); g.clearRect(0, 0, W, H); }
    else if (o === 'clear') { g.fillStyle = p[1]; g.fillRect(0, 0, W, H); }
    else if (o === 'rect') { g.fillStyle = p[5]; g.fillRect(+p[1], +p[2], +p[3], +p[4]); }
    else if (o === 'circle') { g.fillStyle = p[4]; g.beginPath(); g.arc(+p[1], +p[2], +p[3], 0, 2 * Math.PI); g.fill(); }
    else if (o === 'line') { g.strokeStyle = p[6]; g.lineWidth = +p[5]; g.beginPath(); g.moveTo(+p[1], +p[2]); g.lineTo(+p[3], +p[4]); g.stroke(); }
    else if (o === 'text') { g.fillStyle = p[4]; g.font = (+p[3]) + 'px ' + FONT; g.textBaseline = 'middle'; g.fillText(p.slice(5).join(' '), +p[1], +p[2] + (+p[3]) * LINE / 2); }
  }
  st.textContent = status;
  // events: each one once. a page that is new, or that outlived its program, starts at last
  if (last >= 0 && (ack === 'new' || ack > last)) ack = last;
  for (var j = 0; j < fresh.length; j++) { if (ack !== 'new' && fresh[j][0] > ack) { ack = fresh[j][0]; onEvent(fresh[j][1]); } }
  if (want) { mWant = Date.now(); if (mState === 'off') mStart(); }
  else if (mState !== 'off' && Date.now() - mWant > 1500) mStop();
}
var ack = 'new', ended = false;
// what the world says once: quit ends the page (sounds come here later)
function onEvent(p) {
  if (p[0] === 'quit') {
    ended = true;
    // tell the program that the event arrived, then go
    fetch('/frame', {method: 'POST', body: 'ack=' + ack}).catch(function () {}).then(function () {
      document.body.innerHTML = '<div style="color:#ddd;font:20px monospace;padding:40px">The game has ended. You can close this tab.</div>';
      window.close();
    });
  }
}
function again() { if (!ended) requestAnimationFrame(loop); }
function lost() { if (ended) return; st.textContent = 'lust is not running'; setTimeout(loop, 500); }
function loop() {
  var body = 'down=' + Array.from(down).join(',') + ';hit=' + Array.from(hits).join(',') + ';mouse=' + mx.toFixed(1) + ',' + my.toFixed(1) + ',' + mb + ';cw=' + cw.toFixed(4) + ';mic=' + mRead().toFixed(3) + ';ack=' + ack;
  hits.clear();
  fetch('/frame', {method: 'POST', body: body}).then(function (r) { return r.text(); }).then(function (t) { draw(t); again(); }, lost);
}
// a closed page says so; a hidden page (no frames) says it is still there
addEventListener('pagehide', function () { navigator.sendBeacon('/bye', ''); });
// (after quit the program is gone already; the beacon then goes nowhere, which is fine)
setInterval(function () { if (document.hidden) fetch('/ping', {method: 'POST', keepalive: true}).catch(function () {}); }, 5000);
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
            ("POST", "/ping") => {
                ex.lock().unwrap_or_else(|p| p.into_inner()).ping();
                respond(&mut out, "200 OK", "text/plain", "")?;
            }
            ("POST", "/bye") => {
                ex.lock().unwrap_or_else(|p| p.into_inner()).closed();
                respond(&mut out, "200 OK", "text/plain", "")?;
            }
            ("POST", "/frame") => {
                let text = {
                    let mut e = ex.lock().unwrap_or_else(|p| p.into_inner());
                    let input = String::from_utf8_lossy(&body);
                    e.put_input(&input);
                    let (events, last) = e.events_after(screen::ack_of(&input));
                    let mut t = format!("status {}\nevents {}\n", e.status.replace('\n', " "), last);
                    for (n, text) in events {
                        t.push_str(&format!("event {} {}\n", n, text));
                    }
                    t.push_str(&e.frame);
                    t
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
        let body = "down=up;hit=space;mouse=1,2,0;cw=0.55;mic=0.4";
        let f = ask(addr, &format!("POST /frame HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", host, body.len(), body));
        assert!(f.ends_with("status \nevents 0\nsize 10 10\nrect 1 1 2 2 #fff\n"), "{}", f);
        let i = ex.lock().unwrap().take_input();
        assert!(i.down.contains("up") && i.pressed.contains("space"));
        assert_eq!(i.glyph, 0.55);
        assert_eq!(i.mic, 0.4);
        // an event comes after the ack of the page, once
        ex.lock().unwrap().push_event("quit");
        let post = |body: &str| ask(addr, &format!("POST /frame HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", host, body.len(), body));
        assert!(post("ack=new").contains("status \nevents 1\nsize"), "a new page gets no old event");
        assert!(post("ack=0").contains("events 1\nevent 1 quit\nsize"));
        assert!(post("ack=1").contains("events 1\nsize"));
        assert_eq!(ex.lock().unwrap().acked, 1);
        // a request for another host (a page elsewhere, dns rebinding) is refused
        let bad = ask(addr, "GET / HTTP/1.1\r\nHost: evil.example:80\r\nConnection: close\r\n\r\n");
        assert!(bad.starts_with("HTTP/1.1 403"));
    }
}
