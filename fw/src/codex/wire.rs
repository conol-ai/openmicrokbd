//! The Codex Micro / Work Louder protocol codec: report framing, a tiny JSON
//! scanner and writer, the host->device request handler and the
//! device->host event and notification encoders. Pure `core` with no I/O,
//! so it is unit-tested on the host (`scripts/test-codex-wire.sh`) against
//! the request shapes the reference projects' host probes send, the Codex
//! desktop app emits, and the Input app's device kit uses. See `mod.rs` for
//! the protocol overview and provenance.

// ---- report layout ----------------------------------------------------------

pub const REPORT_ID: u8 = 6;
/// Whole report on the wire: report ID + 63-byte body.
pub const REPORT_LEN: usize = 64;
/// Body byte 0: the device kit calls this the channel (1 = debug log, 2 =
/// RPC).
pub const MSG_TYPE: u8 = 2;
pub const PAYLOAD_MAX: usize = 61;
/// Accumulated host request cap. A six-slot `v.oai.thstatus` is ~350
/// bytes; file chunks are streamed past the buffer (`Reassembler`), so
/// anything that has not closed by this size is dropped.
pub const RX_CAP: usize = 1024;
/// One outgoing message (event or reply). Sized for an `fs.readbin` chunk
/// (`READ_CHUNK` bytes as base64) plus its envelope.
pub const TX_CAP: usize = 640;
/// Raw bytes per `fs.readbin` reply. The host asks for 3072 but accepts
/// less and keeps asking.
pub const READ_CHUNK: usize = 384;

/// Vendor-defined collection: 63-byte Input + 63-byte Output, Report ID 6.
#[rustfmt::skip]
pub const REPORT_DESC: &[u8] = &[
    0x06, 0x00, 0xFF, // Usage Page (Vendor Defined 0xFF00)
    0x09, 0x01,       // Usage (1)
    0xA1, 0x01,       // Collection (Application)
    0x85, 0x06,       //   Report ID (6)
    0x15, 0x00,       //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x75, 0x08,       //   Report Size (8)
    0x95, 0x3F,       //   Report Count (63)
    0x09, 0x01,       //   Usage (1)
    0x81, 0x02,       //   Input (Data, Var, Abs)
    0x95, 0x3F,       //   Report Count (63)
    0x09, 0x02,       //   Usage (2)
    0x91, 0x02,       //   Output (Data, Var, Abs)
    0xC0,             // End Collection
];

// ---- device -> host events --------------------------------------------------

pub const ACT_RELEASE: u8 = 0;
pub const ACT_PRESS: u8 = 1;
pub const ACT_STEP: u8 = 2;

/// Which control an event comes from. `Position(p)` is the key at matrix
/// position `p` (0..=12): the official numbering runs in the same reading
/// order as ours, so p0..p5 are the six Agent Keys `AG00`..`AG05` and
/// p6..p12 the Command Keys `ACT06`..`ACT12` (`ACT10`/`ACT11` = the two
/// switches under the wide Mic key, `ACT12` = Send).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Position(u8),
    EncCw,
    EncCcw,
    EncPress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key {
        key: Key,
        act: u8,
    },
    /// Stick direction 0 up / 1 down / 2 left / 3 right (main.rs order).
    Stick {
        dir: u8,
        pressed: bool,
    },
    /// `kb.cs.*` to the Input app: 0 hide, 1 show, 2 toggle.
    CheatSheet {
        mode: u8,
        layer: u8,
        profile: u8,
    },
    /// `kb.radial` to the Input app: the stick as a radial menu — angle in
    /// thousandths of a turn, open while deflected.
    Radial {
        angle_milli: u16,
        open: bool,
        layer: u8,
        profile: u8,
    },
    /// `kb.sa.*` to the Input app: smart action by id (payload looked up in
    /// `smart_actions.json` when sent).
    Smart(u16),
}

// ---- host -> device lighting state -----------------------------------------

/// Lighting effects, as the host's device kit numbers them (`e` on the
/// wire is this integer; the BLE emulators' probes and `lights.preview`
/// send the names instead and both are accepted).
pub const EFFECT_OFF: u8 = 0;
pub const EFFECT_SOLID: u8 = 1;
/// A coloured segment travels along the strip.
pub const EFFECT_SNAKE: u8 = 2;
/// Cycles the full hue wheel (the colour is ignored).
pub const EFFECT_RAINBOW: u8 = 3;
pub const EFFECT_BREATH: u8 = 4;
/// A hue gradient across the strip (the colour is ignored).
pub const EFFECT_GRADIENT: u8 = 5;
/// Breath between half and full brightness.
pub const EFFECT_SHALLOW_BREATH: u8 = 6;

/// One light as the host describes it: 24-bit colour, brightness 0..=255
/// (from the 0..1 multiplier on the wire), an effect and its speed in
/// thousandths (0 = stopped, 1000 = fastest). Every field is optional on
/// the wire and keeps its previous value when absent, as in the references;
/// `set` flips once the host has described the light at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Light {
    pub rgb: u32,
    pub level: u8,
    pub effect: u8,
    pub speed_milli: i32,
    pub set: bool,
}

impl Light {
    pub const OFF: Light = Light {
        rgb: 0,
        level: 0,
        effect: EFFECT_OFF,
        speed_milli: 0,
        set: false,
    };

    /// How far this light's animation clock advances per frame.
    pub fn speed(&self) -> u32 {
        self.speed_milli.clamp(0, 1000) as u32
    }
}

fn effect_from_name(name: &[u8]) -> Option<u8> {
    Some(match name {
        b"off" => EFFECT_OFF,
        b"solid" => EFFECT_SOLID,
        b"snake" => EFFECT_SNAKE,
        b"rainbow" => EFFECT_RAINBOW,
        b"breath" => EFFECT_BREATH,
        b"gradient" => EFFECT_GRADIENT,
        b"shallowBreath" => EFFECT_SHALLOW_BREATH,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lights {
    /// The six Agent Keys (`v.oai.thstatus`), indexed by agent slot = key
    /// position 0..=5.
    pub agents: [Light; 6],
    /// Command-key backlight (`v.oai.rgbcfg` → `keys`, `lights.preview` →
    /// `backlight`).
    pub keys: Light,
    /// Underglow (`v.oai.rgbcfg` → `ambient`, `lights.preview` →
    /// `underglow`).
    pub ambient: Light,
}

impl Lights {
    pub const OFF: Lights = Lights {
        agents: [Light::OFF; 6],
        keys: Light::OFF,
        ambient: Light::OFF,
    };
}

/// Apply one `{c,b,e,s}` (or `{color,brightness,effect,speed}`) object.
pub fn update_light(light: &mut Light, obj: &[u8]) {
    let key = |short: &[u8], long: &[u8]| find_key(obj, short).or_else(|| find_key(obj, long));
    if let Some(c) = key(b"c", b"color").and_then(parse_u32) {
        light.rgb = c & 0x00FF_FFFF;
    }
    if let Some(b) = key(b"b", b"brightness").and_then(parse_milli) {
        light.level = (b.clamp(0, 1000) * 255 / 1000) as u8;
    }
    if let Some(e) = key(b"e", b"effect") {
        if let Some(n) = parse_u32(e) {
            light.effect = n.min(u8::MAX as u32) as u8;
        } else if let Some(effect) = as_str(e).and_then(effect_from_name) {
            light.effect = effect;
        }
    }
    if let Some(s) = key(b"s", b"speed").and_then(parse_milli) {
        light.speed_milli = s;
    }
    light.set = true;
}

// ---- files ------------------------------------------------------------------

/// What the RPC layer needs from the device's file storage (`fs.*`).
pub trait FileStore {
    /// Visit every file present: name, size, SHA-1 digest.
    fn each_file(&mut self, f: &mut dyn FnMut(&[u8], usize, &[u8; 20]));
    /// Whole file, if present. Static: it lives in flash (or in the
    /// firmware image) and outlives any request.
    fn read(&self, name: &[u8]) -> Option<&'static [u8]>;
    /// Start writing `name` from byte 0. False = no such file slot.
    fn begin_write(&mut self, name: &[u8]) -> bool;
    /// Append bytes to the write in progress.
    fn write(&mut self, data: &[u8]) -> bool;
    /// Close the write in progress and publish the file.
    fn finish_write(&mut self) -> bool;
    fn abort_write(&mut self);
    fn delete(&mut self, name: &[u8]) -> bool;
}

// ---- base64 -----------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_val(c: u8) -> Option<u8> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    })
}

/// Incremental base64 decoder: feed any split of the text, get bytes.
#[derive(Default)]
pub struct B64Decode {
    quad: [u8; 4],
    n: u8,
}

impl B64Decode {
    pub const fn new() -> Self {
        Self { quad: [0; 4], n: 0 }
    }

    pub fn reset(&mut self) {
        self.n = 0;
    }

    /// Decode `input`, handing complete bytes to `out` in small runs.
    /// Whitespace and padding are skipped; any other byte is an error.
    pub fn feed(&mut self, input: &[u8], out: &mut dyn FnMut(&[u8])) -> bool {
        let mut buf = [0u8; 48];
        let mut fill = 0usize;
        for &c in input {
            if c == b'=' || c == b'\r' || c == b'\n' || c == b' ' {
                continue;
            }
            let Some(v) = b64_val(c) else {
                return false;
            };
            self.quad[self.n as usize] = v;
            self.n += 1;
            if self.n == 4 {
                let q = self.quad;
                buf[fill] = (q[0] << 2) | (q[1] >> 4);
                buf[fill + 1] = (q[1] << 4) | (q[2] >> 2);
                buf[fill + 2] = (q[2] << 6) | q[3];
                fill += 3;
                self.n = 0;
                if fill == 48 {
                    out(&buf);
                    fill = 0;
                }
            }
        }
        if fill > 0 {
            out(&buf[..fill]);
        }
        true
    }

    /// Flush a trailing partial quad (unpadded input).
    pub fn finish(&mut self, out: &mut dyn FnMut(&[u8])) {
        let q = self.quad;
        match self.n {
            2 => out(&[(q[0] << 2) | (q[1] >> 4)]),
            3 => out(&[(q[0] << 2) | (q[1] >> 4), (q[1] << 4) | (q[2] >> 2)]),
            _ => {}
        }
        self.n = 0;
    }
}

// ---- framing ----------------------------------------------------------------

/// What feeding one report to the reassembler produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    /// Not a frame, or the request is not complete yet.
    Pending,
    /// A complete request of this many bytes sits at the buffer start.
    Complete(usize),
    /// The accumulated request outgrew `RX_CAP` and was discarded.
    Dropped,
}

/// Receives the `data` string of an `fs.writebin` request as it streams
/// past the reassembler, so a 4 KB chunk never has to fit in RAM.
pub trait StreamSink {
    fn start(&mut self, file: &[u8]);
    fn bytes(&mut self, b64: &[u8]);
    fn end(&mut self);
}

/// For callers with nothing to stream to (tests, the OpenMicro-only path).
#[allow(dead_code)]
pub struct NoSink;

impl StreamSink for NoSink {
    fn start(&mut self, _file: &[u8]) {}
    fn bytes(&mut self, _b64: &[u8]) {}
    fn end(&mut self) {}
}

const WRITEBIN_PREFIX: &[u8] = b"{\"method\":\"fs.writebin\",\"params\":{\"file\":\"";
const DATA_KEY: &[u8] = b"\",\"data\":\"";

/// Reassembles host fragments into one JSON request, reference rules:
/// optional leading report-ID byte, type byte 2, length ≤ 61, a fresh
/// `{"method"` prefix resets a stale partial, leading garbage before `{` is
/// skipped, and the buffer is cleared once a complete object is handled.
///
/// `fs.writebin` is special-cased: once the buffer holds the request up to
/// `"data":"`, the base64 body is streamed to the sink instead of being
/// buffered, and the request completes with `"data":""` in its place.
pub struct Reassembler {
    buf: [u8; RX_CAP],
    len: usize,
    streaming: bool,
    /// The current request's data string has already been streamed, so
    /// the prefix must not trigger again for its envelope.
    streamed: bool,
}

impl Reassembler {
    pub const fn new() -> Self {
        Self {
            buf: [0; RX_CAP],
            len: 0,
            streaming: false,
            streamed: false,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.streaming = false;
        self.streamed = false;
    }

    /// Feed one report (with or without its leading report-ID byte).
    pub fn push(&mut self, pkt: &[u8], sink: &mut dyn StreamSink) -> Push {
        if pkt.len() < 2 {
            return Push::Pending;
        }
        let off = if pkt.len() >= 3 && pkt[0] == REPORT_ID {
            1
        } else {
            0
        };
        if pkt.len() < off + 2 || pkt[off] != MSG_TYPE {
            return Push::Pending;
        }
        let plen = (pkt[off + 1] as usize).min(PAYLOAD_MAX);
        if pkt.len() < off + 2 + plen || plen == 0 {
            return Push::Pending;
        }
        let payload = &pkt[off + 2..off + 2 + plen];

        if self.streaming {
            return self.stream_bytes(payload, sink);
        }

        const PREFIX: &[u8] = b"{\"method\"";
        if self.len > 0 && payload.starts_with(PREFIX) {
            // A new top-level request means the previous one was cut short.
            self.len = 0;
            self.streamed = false;
        }
        let payload = if self.len == 0 {
            match payload.iter().position(|&c| c == b'{') {
                Some(start) => &payload[start..],
                None => return Push::Pending,
            }
        } else {
            payload
        };
        if self.len + payload.len() > RX_CAP {
            self.len = 0;
            return Push::Dropped;
        }
        self.buf[self.len..self.len + payload.len()].copy_from_slice(payload);
        self.len += payload.len();

        // An fs.writebin whose data string has opened: hand what follows
        // to the sink and keep only the envelope.
        let head = &self.buf[..self.len];
        if !self.streamed && head.starts_with(WRITEBIN_PREFIX) {
            let name_start = WRITEBIN_PREFIX.len();
            if let Some(q) = find(&head[name_start..], DATA_KEY) {
                let name_end = name_start + q;
                let data_start = name_end + DATA_KEY.len();
                let mut tail = [0u8; PAYLOAD_MAX];
                let n = (self.len - data_start).min(PAYLOAD_MAX);
                tail[..n].copy_from_slice(&self.buf[data_start..data_start + n]);
                let mut name = [0u8; 32];
                let nl = (name_end - name_start).min(32);
                name[..nl].copy_from_slice(&self.buf[name_start..name_start + nl]);
                self.len = data_start;
                self.streaming = true;
                self.streamed = true;
                sink.start(&name[..nl]);
                return self.stream_bytes(&tail[..n], sink);
            }
        }
        match complete_object(&self.buf[..self.len]) {
            Some(n) => Push::Complete(n),
            None => Push::Pending,
        }
    }

    fn stream_bytes(&mut self, bytes: &[u8], sink: &mut dyn StreamSink) -> Push {
        match bytes.iter().position(|&c| c == b'"') {
            None => {
                sink.bytes(bytes);
                Push::Pending
            }
            Some(q) => {
                sink.bytes(&bytes[..q]);
                sink.end();
                self.streaming = false;
                let rest = &bytes[q..];
                if self.len + rest.len() > RX_CAP {
                    self.len = 0;
                    return Push::Dropped;
                }
                self.buf[self.len..self.len + rest.len()].copy_from_slice(rest);
                self.len += rest.len();
                match complete_object(&self.buf[..self.len]) {
                    Some(n) => Push::Complete(n),
                    None => Push::Pending,
                }
            }
        }
    }

    pub fn data(&self, len: usize) -> &[u8] {
        &self.buf[..len]
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Length of the complete top-level `{…}` at the start of `b`, if closed.
fn complete_object(b: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Build the fragment of `json` (newline-terminated on the wire) that
/// starts at byte `off` into `rep`, a full report with the report ID.
/// Returns how many payload bytes it carried; 0 once everything is sent.
#[allow(dead_code)]
pub fn frame(json: &[u8], off: usize, rep: &mut [u8; REPORT_LEN]) -> usize {
    frame_parts(&[json], off, rep)
}

/// `frame` over the concatenation of `parts` — a reply or notification
/// whose middle is a file living in flash need not be assembled first.
pub fn frame_parts(parts: &[&[u8]], off: usize, rep: &mut [u8; REPORT_LEN]) -> usize {
    let json_len: usize = parts.iter().map(|p| p.len()).sum();
    let total = json_len + 1;
    if off >= total {
        return 0;
    }
    let chunk = (total - off).min(PAYLOAD_MAX);
    rep.fill(0);
    rep[0] = REPORT_ID;
    rep[1] = MSG_TYPE;
    rep[2] = chunk as u8;
    // Locate the part holding `off`, then stream forward.
    let mut part = 0usize;
    let mut idx = off;
    while part < parts.len() && idx >= parts[part].len() {
        idx -= parts[part].len();
        part += 1;
    }
    for k in 0..chunk {
        rep[3 + k] = if part < parts.len() {
            let b = parts[part][idx];
            idx += 1;
            while part < parts.len() && idx >= parts[part].len() {
                idx = 0;
                part += 1;
            }
            b
        } else {
            b'\n'
        };
    }
    chunk
}

/// The writebin envelope's `file` and flags once the data has streamed
/// past: streamed writes carry `"data":""`.
pub struct WriteState {
    pub dec: B64Decode,
    /// A slot write is open (first chunk seen, not yet completed).
    pub active: bool,
    pub file: [u8; 32],
    pub file_len: u8,
    /// Decoded bytes in the current chunk.
    pub chunk: usize,
    pub ok: bool,
}

impl WriteState {
    pub const fn new() -> Self {
        Self {
            dec: B64Decode::new(),
            active: false,
            file: [0; 32],
            file_len: 0,
            chunk: 0,
            ok: true,
        }
    }
}

/// Decodes the streamed base64 into the store: the first chunk of a file
/// (or a different file) starts a new write.
pub struct WriteSink<'a> {
    pub state: &'a mut WriteState,
    pub store: &'a mut dyn FileStore,
}

impl StreamSink for WriteSink<'_> {
    fn start(&mut self, file: &[u8]) {
        let st = &mut *self.state;
        let same = st.active && &st.file[..st.file_len as usize] == file;
        st.chunk = 0;
        st.dec.reset();
        if same {
            st.ok = true;
            return;
        }
        st.file = [0; 32];
        let n = file.len().min(32);
        st.file[..n].copy_from_slice(&file[..n]);
        st.file_len = n as u8;
        st.ok = self.store.begin_write(file);
        st.active = st.ok;
    }

    fn bytes(&mut self, b64: &[u8]) {
        if !self.state.ok {
            return;
        }
        let store = &mut *self.store;
        let mut ok = true;
        let mut n = 0usize;
        let fed = self.state.dec.feed(b64, &mut |bytes| {
            if ok {
                ok = store.write(bytes);
                n += bytes.len();
            }
        });
        self.state.chunk += n;
        self.state.ok = fed && ok;
    }

    fn end(&mut self) {
        if !self.state.ok {
            return;
        }
        let store = &mut *self.store;
        let mut ok = true;
        let mut n = 0usize;
        self.state.dec.finish(&mut |bytes| {
            if ok {
                ok = store.write(bytes);
                n += bytes.len();
            }
        });
        self.state.chunk += n;
        self.state.ok = ok;
    }
}

// ---- a very small JSON scanner ----------------------------------------------

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    i
}

/// `b[i] == '"'` → index just past the closing quote.
fn skip_string(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let mut esc = false;
    while j < b.len() {
        let c = b[j];
        if esc {
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else if c == b'"' {
            return Some(j + 1);
        }
        j += 1;
    }
    None
}

/// Index just past the value starting at (or after whitespace from) `i`.
fn skip_value(b: &[u8], i: usize) -> Option<usize> {
    let i = skip_ws(b, i);
    match *b.get(i)? {
        b'"' => skip_string(b, i),
        b'{' | b'[' => {
            let mut depth = 0i32;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = skip_string(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            // number / true / false / null: runs to a delimiter
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']') && !is_ws(b[j]) {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

/// Raw value slice of member `key` in the object `obj` (which starts with
/// `{`, after optional whitespace). Escapes in keys are not decoded — none
/// of the protocol's keys need them.
pub fn find_key<'a>(obj: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut i = skip_ws(obj, 0);
    if obj.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    loop {
        i = skip_ws(obj, i);
        match *obj.get(i)? {
            b'}' => return None,
            b',' => {
                i += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let kend = skip_string(obj, i)?;
        let k = &obj[i + 1..kend - 1];
        i = skip_ws(obj, kend);
        if obj.get(i) != Some(&b':') {
            return None;
        }
        i = skip_ws(obj, i + 1);
        let vend = skip_value(obj, i)?;
        if k == key {
            return Some(&obj[i..vend]);
        }
        i = vend;
    }
}

/// Call `f` on every element of the array `arr` (which starts with `[`).
/// Elements borrow from `arr`, so a callback may keep one.
pub fn for_each_elem<'a>(arr: &'a [u8], mut f: impl FnMut(&'a [u8])) {
    let mut i = skip_ws(arr, 0);
    if arr.get(i) != Some(&b'[') {
        return;
    }
    i += 1;
    loop {
        i = skip_ws(arr, i);
        match arr.get(i) {
            None | Some(b']') => return,
            Some(b',') => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let Some(vend) = skip_value(arr, i) else {
            return;
        };
        f(&arr[i..vend]);
        i = vend;
    }
}

pub fn is_object(v: &[u8]) -> bool {
    v.first() == Some(&b'{')
}

pub fn is_array(v: &[u8]) -> bool {
    v.first() == Some(&b'[')
}

/// Contents of a string value (undecoded).
pub fn as_str(v: &[u8]) -> Option<&[u8]> {
    if v.len() >= 2 && v[0] == b'"' && v[v.len() - 1] == b'"' {
        Some(&v[1..v.len() - 1])
    } else {
        None
    }
}

/// Non-negative integer value; a fractional part is ignored (`255.0`).
pub fn parse_u32(v: &[u8]) -> Option<u32> {
    let mut n: u32 = 0;
    let mut any = false;
    for &c in v {
        match c {
            b'0'..=b'9' => {
                n = n.checked_mul(10)?.checked_add((c - b'0') as u32)?;
                any = true;
            }
            b'.' => break,
            _ => return None,
        }
    }
    any.then_some(n)
}

/// Decimal number in thousandths: `1` → 1000, `0.4` → 400, `-0.25` → -250.
/// Exponents are not expected on this protocol and yield `None`.
pub fn parse_milli(v: &[u8]) -> Option<i32> {
    let (neg, digits) = match v.first()? {
        b'-' => (true, &v[1..]),
        _ => (false, v),
    };
    let mut whole: i32 = 0;
    let mut frac: i32 = 0;
    let mut scale: i32 = 1000;
    let mut in_frac = false;
    let mut any = false;
    for &c in digits {
        match c {
            b'0'..=b'9' if !in_frac => {
                whole = whole.saturating_mul(10).saturating_add((c - b'0') as i32);
                any = true;
            }
            b'0'..=b'9' => {
                if scale > 1 {
                    scale /= 10;
                    frac += (c - b'0') as i32 * scale;
                }
            }
            b'.' if !in_frac => in_frac = true,
            _ => return None,
        }
    }
    if !any {
        return None;
    }
    let m = whole.saturating_mul(1000).saturating_add(frac);
    Some(if neg { -m } else { m })
}

fn parse_bool(v: &[u8]) -> Option<bool> {
    match v {
        b"true" | b"1" => Some(true),
        b"false" | b"0" => Some(false),
        _ => None,
    }
}

// ---- a very small JSON writer -----------------------------------------------

pub struct Out<'a> {
    buf: &'a mut [u8],
    len: usize,
    overflow: bool,
}

impl<'a> Out<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            overflow: false,
        }
    }

    pub fn push(&mut self, s: &[u8]) {
        if self.len + s.len() > self.buf.len() {
            self.overflow = true;
            return;
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }

    pub fn push_u32(&mut self, mut v: u32) {
        let mut tmp = [0u8; 10];
        let mut i = tmp.len();
        loop {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.push(&tmp[i..]);
    }

    /// Thousandths as a decimal: 750 → `0.75`, 0 → `0`, 1000 → `1`.
    pub fn push_milli(&mut self, m: u16) {
        let m = m.min(1000);
        self.push_u32((m / 1000) as u32);
        let frac = m % 1000;
        if frac != 0 {
            let digits = [
                b'0' + (frac / 100) as u8,
                b'0' + (frac / 10 % 10) as u8,
                b'0' + (frac % 10) as u8,
            ];
            let mut n = 3;
            while n > 1 && digits[n - 1] == b'0' {
                n -= 1;
            }
            self.push(b".");
            let d = digits[..n].to_owned_array();
            self.push(&d.0[..d.1]);
        }
    }

    pub fn push_base64(&mut self, data: &[u8]) {
        for chunk in data.chunks(3) {
            let b0 = chunk[0];
            let b1 = chunk.get(1).copied().unwrap_or(0);
            let b2 = chunk.get(2).copied().unwrap_or(0);
            let mut q = [
                B64[(b0 >> 2) as usize],
                B64[(((b0 & 3) << 4) | (b1 >> 4)) as usize],
                B64[(((b1 & 15) << 2) | (b2 >> 6)) as usize],
                B64[(b2 & 63) as usize],
            ];
            if chunk.len() < 3 {
                q[3] = b'=';
            }
            if chunk.len() < 2 {
                q[2] = b'=';
            }
            self.push(&q);
        }
    }

    pub fn push_hex20(&mut self, digest: &[u8; 20]) {
        let mut h = [0u8; 40];
        super::sha1::hex(digest, &mut h);
        self.push(&h);
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn finish(self) -> usize {
        if self.overflow {
            0
        } else {
            self.len
        }
    }
}

/// Tiny helper so `push_milli` can slice a copied array without borrowing
/// `self` twice.
trait OwnedArray {
    fn to_owned_array(&self) -> ([u8; 3], usize);
}

impl OwnedArray for [u8] {
    fn to_owned_array(&self) -> ([u8; 3], usize) {
        let mut a = [0u8; 3];
        let n = self.len().min(3);
        a[..n].copy_from_slice(&self[..n]);
        (a, n)
    }
}

// ---- RPC --------------------------------------------------------------------

fn apply_thstatus(lights: &mut Lights, arr: &[u8]) {
    for_each_elem(arr, |el| {
        let Some(id) = find_key(el, b"id").and_then(parse_u32) else {
            return;
        };
        if id < 6 {
            update_light(&mut lights.agents[id as usize], el);
        }
    });
}

fn apply_rgbcfg(lights: &mut Lights, params: &[u8]) {
    if let Some(a) = find_key(params, b"ambient").filter(|v| is_object(v)) {
        update_light(&mut lights.ambient, a);
    }
    if let Some(k) = find_key(params, b"keys").filter(|v| is_object(v)) {
        update_light(&mut lights.keys, k);
    }
}

/// `lights.preview` from the Input app: `{"backlight":{…},"underglow":{…}}`
/// with the same fields under their long names.
fn apply_preview(lights: &mut Lights, params: &[u8]) {
    if let Some(b) = find_key(params, b"backlight").filter(|v| is_object(v)) {
        update_light(&mut lights.keys, b);
    }
    if let Some(u) = find_key(params, b"underglow").filter(|v| is_object(v)) {
        update_light(&mut lights.ambient, u);
    }
}

/// `{"id":<echoed>,` — the id is copied verbatim (number or string); a
/// request without one gets `null`, as the references' serialiser would.
fn open_reply(out: &mut Out, id: Option<&[u8]>) {
    out.push(b"{\"id\":");
    match id {
        Some(id) if id.len() <= 40 => out.push(id),
        _ => out.push(b"null"),
    }
    out.push(b",");
}

fn ok_reply(out: &mut Out, id: Option<&[u8]>) {
    open_reply(out, id);
    out.push(b"\"result\":{\"ok\":true}}");
}

fn error_reply(out: &mut Out, id: Option<&[u8]>, code: i32, message: &[u8]) {
    open_reply(out, id);
    out.push(b"\"error\":{\"code\":");
    if code < 0 {
        out.push(b"-");
        out.push_u32(code.unsigned_abs());
    } else {
        out.push_u32(code as u32);
    }
    out.push(b",\"message\":\"");
    out.push(message);
    out.push(b"\"}}");
}

/// Which request a complete message turned out to be (for logging).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    SysVersion,
    DeviceStatus,
    ThreadStatus,
    RgbConfig,
    LightsPreview,
    /// `fs.*`
    File,
    /// A `fs.writebin` / `fs.write` that completed a file, or a `fs.delete`
    /// that removed one: the caller
    /// should reload anything derived from it.
    FileWritten,
    Acked,
    /// Unknown method — or none at all: the references default a missing
    /// method to "" and answer `Method not found` with the id, so a caller
    /// waiting on that id never hangs.
    Unknown,
}

/// What to send back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Nothing (the reply did not fit `tx`).
    None,
    /// `tx[..n]`.
    Buf(usize),
    /// `tx[..head]`, then the whole file `name`, then `tail`.
    File {
        head: usize,
        name: [u8; 32],
        name_len: u8,
        tail: &'static [u8],
    },
}

/// Profile / layer the pad currently runs, for `device.status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub profile_index: u8,
    pub layer_index: u8,
}

/// Everything a request handler may touch.
pub struct Ctx<'a> {
    pub version: &'a str,
    pub lights: &'a mut Lights,
    pub store: &'a mut dyn FileStore,
    pub write: &'a mut WriteState,
    pub status: Status,
}

fn file_name(v: &[u8]) -> Option<&[u8]> {
    let name = as_str(v)?;
    Some(name.strip_prefix(b"/").unwrap_or(name))
}

/// Handle one complete request: update `ctx`, write the reply to `tx`.
pub fn handle_request(req: &[u8], ctx: &mut Ctx, tx: &mut [u8]) -> (Handled, Reply) {
    let method = find_key(req, b"method").and_then(as_str).unwrap_or(b"");
    let id = find_key(req, b"id");
    let params = find_key(req, b"params");
    let mut out = Out::new(tx);

    let what = match method {
        b"sys.version" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"version\":\"");
            out.push(ctx.version.as_bytes());
            out.push(b"\"}}");
            Handled::SysVersion
        }
        b"device.status" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"version\":\"");
            out.push(ctx.version.as_bytes());
            out.push(b"\",\"profile_index\":");
            out.push_u32(ctx.status.profile_index as u32);
            out.push(b",\"layer_index\":");
            out.push_u32(ctx.status.layer_index as u32);
            // Wired: always "full", never charging.
            out.push(b",\"battery\":100,\"is_charging\":false}}");
            Handled::DeviceStatus
        }
        b"v.oai.thstatus" if params.is_some_and(is_array) => {
            apply_thstatus(ctx.lights, params.unwrap_or(b"[]"));
            ok_reply(&mut out, id);
            Handled::ThreadStatus
        }
        b"v.oai.rgbcfg" if params.is_some_and(is_object) => {
            apply_rgbcfg(ctx.lights, params.unwrap_or(b"{}"));
            ok_reply(&mut out, id);
            Handled::RgbConfig
        }
        b"lights.preview" => {
            if let Some(p) = params.filter(|v| is_object(v)) {
                apply_preview(ctx.lights, p);
            }
            ok_reply(&mut out, id);
            Handled::LightsPreview
        }
        b"fs.list" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":[");
            let mut first = true;
            ctx.store.each_file(&mut |name, size, sha| {
                if !first {
                    out.push(b",");
                }
                first = false;
                out.push(b"{\"name\":\"");
                out.push(name);
                out.push(b"\",\"size\":\"");
                out.push_u32(size as u32);
                out.push(b"\",\"checksum\":\"");
                out.push_hex20(sha);
                out.push(b"\"}");
            });
            out.push(b"]}");
            Handled::File
        }
        b"fs.readbin" => {
            let name = params
                .and_then(|p| find_key(p, b"file"))
                .and_then(file_name);
            match name.and_then(|n| ctx.store.read(n)) {
                Some(body) => {
                    let offset = params
                        .and_then(|p| find_key(p, b"offset"))
                        .and_then(parse_u32)
                        .unwrap_or(0) as usize;
                    let want = params
                        .and_then(|p| find_key(p, b"len"))
                        .and_then(parse_u32)
                        .unwrap_or(READ_CHUNK as u32) as usize;
                    let start = offset.min(body.len());
                    let end = (start + want.min(READ_CHUNK)).min(body.len());
                    open_reply(&mut out, id);
                    out.push(b"\"result\":{\"total_size\":");
                    out.push_u32(body.len() as u32);
                    out.push(b",\"data\":\"");
                    out.push_base64(&body[start..end]);
                    out.push(b"\"}}");
                }
                None => error_reply(&mut out, id, -2, b"File does not exist"),
            }
            Handled::File
        }
        b"fs.read" => {
            let name = params
                .and_then(|p| find_key(p, b"file"))
                .and_then(file_name);
            match name.map(|n| (n, ctx.store.read(n))) {
                Some((n, Some(body))) if complete_object(body) == Some(body.len()) => {
                    open_reply(&mut out, id);
                    out.push(b"\"result\":");
                    let head = out.finish();
                    let mut name = [0u8; 32];
                    let nl = n.len().min(32);
                    name[..nl].copy_from_slice(&n[..nl]);
                    return (
                        Handled::File,
                        if head == 0 {
                            Reply::None
                        } else {
                            Reply::File {
                                head,
                                name,
                                name_len: nl as u8,
                                tail: b"}",
                            }
                        },
                    );
                }
                _ => error_reply(&mut out, id, -2, b"File does not exist"),
            }
            Handled::File
        }
        b"fs.writebin" => {
            // The data string streamed past the reassembler into the store
            // (`WriteSink`); only the envelope is here.
            let completed = params
                .and_then(|p| find_key(p, b"completed"))
                .and_then(parse_bool)
                .unwrap_or(false);
            let chunk = ctx.write.chunk;
            let mut what = Handled::File;
            if !ctx.write.active || !ctx.write.ok {
                ctx.store.abort_write();
                ctx.write.active = false;
                error_reply(&mut out, id, -3, b"write failed");
            } else {
                if completed {
                    ctx.write.active = false;
                    if ctx.store.finish_write() {
                        what = Handled::FileWritten;
                    } else {
                        error_reply(&mut out, id, -3, b"write failed");
                        return (Handled::File, buf_reply(out));
                    }
                }
                open_reply(&mut out, id);
                out.push(b"\"result\":{\"data_written\":");
                out.push_u32(chunk as u32);
                out.push(b"}}");
            }
            what
        }
        b"fs.write" => {
            let name = params
                .and_then(|p| find_key(p, b"file"))
                .and_then(file_name);
            let data = params.and_then(|p| find_key(p, b"data"));
            let mut what = Handled::File;
            match (name, data) {
                (Some(n), Some(d)) => {
                    ctx.write.active = false;
                    let ok =
                        ctx.store.begin_write(n) && ctx.store.write(d) && ctx.store.finish_write();
                    if ok {
                        ok_reply(&mut out, id);
                        what = Handled::FileWritten;
                    } else {
                        ctx.store.abort_write();
                        error_reply(&mut out, id, -3, b"write failed");
                    }
                }
                _ => error_reply(&mut out, id, -1, b"params are not correct"),
            }
            what
        }
        b"fs.delete" => {
            let name = params
                .and_then(|p| find_key(p, b"file"))
                .and_then(file_name);
            ctx.write.active = false;
            match name {
                Some(n) if ctx.store.delete(n) => {
                    ok_reply(&mut out, id);
                    // The built-in keymap takes over: reload like a write.
                    Handled::FileWritten
                }
                _ => {
                    error_reply(&mut out, id, -2, b"File does not exist");
                    Handled::File
                }
            }
        }
        b"fs.rmdir" | b"fs.txcommit" => {
            ok_reply(&mut out, id);
            Handled::File
        }
        b"fs.txbegin" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"tx\":1}}");
            Handled::File
        }
        b"ui.active_screen" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"screen_name\":\"home\"}}");
            Handled::Acked
        }
        b"appmgr.list_active" | b"appmgr.list_installed" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":[]}");
            Handled::Acked
        }
        b"host.focused_app"
        | b"ui.home_accent_color"
        | b"mp.write_info"
        | b"mp.write_artwork"
        | b"sys.selftest" => {
            ok_reply(&mut out, id);
            Handled::Acked
        }
        _ => {
            error_reply(&mut out, id, -32601, b"Method not found");
            Handled::Unknown
        }
    };
    (what, buf_reply(out))
}

fn buf_reply(out: Out) -> Reply {
    match out.finish() {
        0 => Reply::None,
        n => Reply::Buf(n),
    }
}

/// Encode one device->host event; returns its length in `tx`.
pub fn event_json(ev: Event, tx: &mut [u8]) -> usize {
    let mut out = Out::new(tx);
    match ev {
        Event::Key { key, act } => {
            out.push(b"{\"method\":\"v.oai.hid\",\"params\":{\"k\":\"");
            match key {
                Key::Position(p) if p < 6 => {
                    out.push(b"AG0");
                    out.push_u32(p as u32);
                }
                Key::Position(p) => {
                    out.push(b"ACT");
                    if p < 10 {
                        out.push(b"0");
                    }
                    out.push_u32(p as u32);
                }
                Key::EncCw => out.push(b"ENC_CW"),
                Key::EncCcw => out.push(b"ENC_CC"),
                Key::EncPress => out.push(b"ENC"),
            }
            out.push(b"\",\"act\":");
            out.push_u32(act as u32);
            if let Key::Position(p) = key {
                if p < 6 {
                    out.push(b",\"ag\":");
                    out.push_u32(p as u32);
                }
            }
            out.push(b"}}");
        }
        Event::Stick { dir, pressed } => {
            out.push(b"{\"method\":\"v.oai.rad\",\"params\":{\"a\":");
            out.push(match dir {
                0 => b"0.75" as &[u8], // up
                1 => b"0.25",          // down
                2 => b"0.5",           // left
                _ => b"0",             // right
            });
            out.push(b",\"d\":");
            out.push(if pressed { b"1" } else { b"0" });
            out.push(b"}}");
        }
        Event::CheatSheet {
            mode,
            layer,
            profile,
        } => {
            out.push(b"{\"method\":\"");
            out.push(match mode {
                0 => b"kb.cs.hide" as &[u8],
                1 => b"kb.cs.show",
                _ => b"kb.cs.toggle",
            });
            out.push(b"\",\"params\":{\"l\":");
            out.push_u32(layer as u32);
            out.push(b",\"p\":");
            out.push_u32(profile as u32);
            out.push(b"}}");
        }
        Event::Radial {
            angle_milli,
            open,
            layer,
            profile,
        } => {
            out.push(b"{\"method\":\"kb.radial\",\"params\":{\"a\":");
            out.push_milli(angle_milli);
            out.push(b",\"d\":");
            out.push(if open { b"1" } else { b"0" });
            out.push(b",\"l\":");
            out.push_u32(layer as u32);
            out.push(b",\"p\":");
            out.push_u32(profile as u32);
            out.push(b",\"o\":");
            out.push(if open { b"1" } else { b"0" });
            out.push(b"}}");
        }
        Event::Smart(_) => {
            // Needs the smart-actions file: see `notify_head`.
        }
    }
    out.finish()
}

/// `{"method":"<method>","params":` — the caller appends the raw params
/// object and a closing brace (used for smart actions, whose payload lives
/// in flash).
pub fn notify_head(method: &[u8], tx: &mut [u8]) -> usize {
    let mut out = Out::new(tx);
    out.push(b"{\"method\":\"");
    out.push(method);
    out.push(b"\",\"params\":");
    out.finish()
}
