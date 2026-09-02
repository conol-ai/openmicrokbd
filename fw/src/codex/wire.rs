//! The Codex Micro protocol codec: report framing, a tiny JSON scanner and
//! writer, the host->device request handler and the device->host event
//! encoder. Pure `core` with no I/O, so it is unit-tested on the host
//! (`scripts/test-codex-wire.sh`) against the request shapes the reference
//! projects' host probes send and their protocol notes describe. See
//! `mod.rs` for the protocol overview and provenance.

// ---- report layout ----------------------------------------------------------

pub const REPORT_ID: u8 = 6;
/// Whole report on the wire: report ID + 63-byte body.
pub const REPORT_LEN: usize = 64;
pub const MSG_TYPE: u8 = 2;
pub const PAYLOAD_MAX: usize = 61;
/// Accumulated host request cap. A six-slot `v.oai.thstatus` is ~350
/// bytes; anything that has not closed by this size is dropped.
pub const RX_CAP: usize = 1024;
/// One outgoing message (event or reply). `device.status` is ~120 bytes.
pub const TX_CAP: usize = 256;

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
}

// ---- host -> device lighting state -----------------------------------------

/// One light as the host describes it: 24-bit colour, brightness 0..=255
/// (from the 0..1 multiplier on the wire), and an animation flag. Every
/// field is optional on the wire and keeps its previous value when absent,
/// as in the references; `set` flips once the host has described it at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Light {
    pub rgb: u32,
    pub level: u8,
    pub effect_breath: bool,
    pub speed_milli: i32,
    pub set: bool,
}

impl Light {
    pub const OFF: Light = Light {
        rgb: 0,
        level: 0,
        effect_breath: false,
        speed_milli: 0,
        set: false,
    };

    /// Observed host behaviour: the focused slot arrives as `e:"off"` with
    /// `s:0.4`; older builds sent `e:"breath"`. Either means "animate".
    pub fn breath(&self) -> bool {
        self.effect_breath || self.speed_milli > 10
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lights {
    /// The six Agent Keys (`v.oai.thstatus`), indexed by agent slot = key
    /// position 0..=5.
    pub agents: [Light; 6],
    /// Command-key backlight (`v.oai.rgbcfg` → `keys`).
    pub keys: Light,
    /// Underglow (`v.oai.rgbcfg` → `ambient`).
    pub ambient: Light,
}

impl Lights {
    pub const OFF: Lights = Lights {
        agents: [Light::OFF; 6],
        keys: Light::OFF,
        ambient: Light::OFF,
    };
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

/// Reassembles host fragments into one JSON request, reference rules:
/// optional leading report-ID byte, type byte 2, length ≤ 61, a fresh
/// `{"method"` prefix resets a stale partial, leading garbage before `{` is
/// skipped, and the buffer is cleared once a complete object is handled.
pub struct Reassembler {
    buf: [u8; RX_CAP],
    len: usize,
}

impl Reassembler {
    pub const fn new() -> Self {
        Self {
            buf: [0; RX_CAP],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Feed one report (with or without its leading report-ID byte).
    pub fn push(&mut self, pkt: &[u8]) -> Push {
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

        const PREFIX: &[u8] = b"{\"method\"";
        if self.len > 0 && payload.starts_with(PREFIX) {
            // A new top-level request means the previous one was cut short.
            self.len = 0;
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
        match complete_object(&self.buf[..self.len]) {
            Some(n) => Push::Complete(n),
            None => Push::Pending,
        }
    }

    pub fn data(&self, len: usize) -> &[u8] {
        &self.buf[..len]
    }
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
pub fn frame(json: &[u8], off: usize, rep: &mut [u8; REPORT_LEN]) -> usize {
    let total = json.len() + 1;
    if off >= total {
        return 0;
    }
    let chunk = (total - off).min(PAYLOAD_MAX);
    rep.fill(0);
    rep[0] = REPORT_ID;
    rep[1] = MSG_TYPE;
    rep[2] = chunk as u8;
    for k in 0..chunk {
        let i = off + k;
        rep[3 + k] = if i < json.len() { json[i] } else { b'\n' };
    }
    chunk
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
fn find_key<'a>(obj: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
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
fn for_each_elem(arr: &[u8], mut f: impl FnMut(&[u8])) {
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

fn is_object(v: &[u8]) -> bool {
    v.first() == Some(&b'{')
}

fn is_array(v: &[u8]) -> bool {
    v.first() == Some(&b'[')
}

/// Contents of a string value (undecoded).
fn as_str(v: &[u8]) -> Option<&[u8]> {
    if v.len() >= 2 && v[0] == b'"' && v[v.len() - 1] == b'"' {
        Some(&v[1..v.len() - 1])
    } else {
        None
    }
}

/// Non-negative integer value; a fractional part is ignored (`255.0`).
fn parse_u32(v: &[u8]) -> Option<u32> {
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
fn parse_milli(v: &[u8]) -> Option<i32> {
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

// ---- a very small JSON writer -----------------------------------------------

struct Out<'a> {
    buf: &'a mut [u8],
    len: usize,
    overflow: bool,
}

impl<'a> Out<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            overflow: false,
        }
    }

    fn push(&mut self, s: &[u8]) {
        if self.len + s.len() > self.buf.len() {
            self.overflow = true;
            return;
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }

    fn push_u32(&mut self, mut v: u32) {
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

    fn finish(self) -> usize {
        if self.overflow {
            0
        } else {
            self.len
        }
    }
}

// ---- RPC --------------------------------------------------------------------

fn update_light(light: &mut Light, obj: &[u8]) {
    if let Some(c) = find_key(obj, b"c").and_then(parse_u32) {
        light.rgb = c & 0x00FF_FFFF;
    }
    if let Some(b) = find_key(obj, b"b").and_then(parse_milli) {
        light.level = (b.clamp(0, 1000) * 255 / 1000) as u8;
    }
    if let Some(e) = find_key(obj, b"e").and_then(as_str) {
        light.effect_breath = e == b"breath";
    }
    if let Some(s) = find_key(obj, b"s").and_then(parse_milli) {
        light.speed_milli = s;
    }
    light.set = true;
}

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

/// Which request a complete message turned out to be (for logging).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    SysVersion,
    DeviceStatus,
    ThreadStatus,
    RgbConfig,
    Acked,
    /// Unknown method — or none at all: the references default a missing
    /// method to "" and answer `Method not found` with the id, so a caller
    /// waiting on that id never hangs.
    Unknown,
}

/// Handle one complete request: update `lights` and write the reply to
/// `tx`. Returns what it was and the reply length (0 only if the reply
/// would not fit `tx`).
pub fn handle_request(
    req: &[u8],
    version: &str,
    lights: &mut Lights,
    tx: &mut [u8],
) -> (Handled, usize) {
    let method = find_key(req, b"method").and_then(as_str).unwrap_or(b"");
    let id = find_key(req, b"id");
    let params = find_key(req, b"params");
    let mut out = Out::new(tx);

    let what = match method {
        b"sys.version" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"version\":\"");
            out.push(version.as_bytes());
            out.push(b"\"}}");
            Handled::SysVersion
        }
        b"device.status" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"version\":\"");
            out.push(version.as_bytes());
            // Wired: always "full", never charging. profile/layer mirror
            // what the references answered.
            out.push(
                b"\",\"profile_index\":0,\"layer_index\":1,\"battery\":100,\"is_charging\":false}}",
            );
            Handled::DeviceStatus
        }
        b"v.oai.thstatus" if params.is_some_and(is_array) => {
            apply_thstatus(lights, params.unwrap_or(b"[]"));
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"ok\":true}}");
            Handled::ThreadStatus
        }
        b"v.oai.rgbcfg" if params.is_some_and(is_object) => {
            apply_rgbcfg(lights, params.unwrap_or(b"{}"));
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"ok\":true}}");
            Handled::RgbConfig
        }
        b"lights.preview" | b"host.focused_app" => {
            open_reply(&mut out, id);
            out.push(b"\"result\":{\"ok\":true}}");
            Handled::Acked
        }
        _ => {
            open_reply(&mut out, id);
            out.push(b"\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}");
            Handled::Unknown
        }
    };
    (what, out.finish())
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
    }
    out.finish()
}
