//! Compiles the firmware's Codex Micro codec (`fw/src/codex/wire.rs`, pure
//! `core`) for the host and exercises it against the request shapes the
//! reference projects' host probes send (their protocol notes are the spec). Run via
//! `scripts/test-codex-wire.sh`.

#[path = "../../src/codex/layout.rs"]
#[allow(dead_code)]
pub mod layout;
#[path = "../../src/codex/sha1.rs"]
#[allow(dead_code)]
pub mod sha1;
#[path = "../../src/codex/wire.rs"]
#[allow(dead_code)]
pub mod wire;

#[cfg(test)]
mod sha1_tests {
    use super::sha1::{digest, hex, Sha1};

    fn hexs(d: &[u8; 20]) -> String {
        let mut h = [0u8; 40];
        hex(d, &mut h);
        String::from_utf8(h.to_vec()).unwrap()
    }

    #[test]
    fn standard_vectors() {
        assert_eq!(
            hexs(&digest(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hexs(&digest(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hexs(&digest(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hexs(&digest(&million)),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    #[test]
    fn streaming_matches_one_shot_at_every_split() {
        let data: Vec<u8> = (0..200u32).map(|i| (i * 7 % 251) as u8).collect();
        let whole = digest(&data);
        for split in [1usize, 3, 55, 56, 63, 64, 65, 100, 128, 199] {
            let mut s = Sha1::new();
            s.update(&data[..split]);
            s.update(&data[split..]);
            assert_eq!(s.finish(), whole, "split {split}");
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::layout::*;

    #[test]
    fn keycodes_map_to_bindings() {
        assert_eq!(
            binding(b"KC_A"),
            Binding::Key {
                mods: 0,
                code: 0x04
            }
        );
        assert_eq!(
            binding(b"KC_F13"),
            Binding::Key {
                mods: 0,
                code: 0x68
            }
        );
        assert_eq!(
            binding(b"KC_LGUI"),
            Binding::Key {
                mods: 0x08,
                code: 0
            }
        );
        assert_eq!(binding(b"KC_MPLY"), Binding::Consumer(0xCD));
        assert_eq!(binding(b"KC_NONE"), Binding::None);
        assert_eq!(binding(b"KC_BOGUS"), Binding::Unsupported);
        assert_eq!(binding(b"KV_OAI_AG03"), Binding::Oai(3));
        assert_eq!(binding(b"KV_OAI_ACT12"), Binding::Oai(12));
        assert_eq!(binding(b"KV_OAI_ENC_CC"), Binding::Oai(OAI_ENC_CCW));
        assert_eq!(binding(b"KV_OAI_ENC_CLK"), Binding::Oai(OAI_ENC_PRESS));
        assert_eq!(binding(b"KI_LS2"), Binding::LayerToggle(2));
        assert_eq!(binding(b"KI_LM1"), Binding::LayerHold(1));
        assert_eq!(binding(b"KI_PS3"), Binding::Profile(2));
        assert_eq!(binding(b"KI_FP"), Binding::Function);
        assert_eq!(binding(b"KC_FUNC"), Binding::Function);
        assert_eq!(binding(b"KA_A7"), Binding::Macro(7));
        assert_eq!(binding(b"KA_M2"), Binding::Multi(2));
        assert_eq!(binding(b"SA_11"), Binding::Smart(11));
        assert_eq!(binding(b"KI_CS_SHOW_TMP"), Binding::CheatSheet(3));
        assert_eq!(binding(b"KI_BLDW"), Binding::Backlight(-1));
        assert_eq!(binding(b"KI_CBT1"), Binding::Unsupported);
    }

    #[test]
    fn default_keymap_is_the_chatgpt_layer() {
        let doc = DEFAULT_KEYMAP.as_bytes();
        let l = parse(doc, None, 0).expect("parses");
        for p in 0..13u8 {
            assert_eq!(l.keys[p as usize], Binding::Oai(p), "key {p}");
        }
        assert_eq!(
            l.encoder,
            [
                Binding::Oai(OAI_ENC_CCW),
                Binding::Oai(OAI_ENC_CW),
                Binding::Oai(OAI_ENC_PRESS)
            ]
        );
        assert_eq!(l.touch, Binding::Consumer(0xCD));
        assert_eq!(l.joystick, Joystick::Vendor);
        assert_eq!(
            (
                l.profile_index,
                l.layer_index,
                l.profile_count,
                l.layer_count
            ),
            (0, 0, 1, 1)
        );
    }

    const TWO_LAYERS: &str = r#"{"version":1,"activeProfileId":5,"profiles":[
      {"id":2,"name":"Other","layers":[{"id":0,"name":"x","color":0,"os":0,"layout":{"keymap":[["KC_X"]],"encoders":[],"joystick":{"type":"VENDOR","sectors":[]}}}]},
      {"id":5,"name":"Work","layers":[
        {"id":0,"name":"Base","color":255,"os":0,"lights":{"backlight":{"effect":"solid","brightness":0.5,"speed":0,"magic":0,"color":65280},"underglow":{"effect":"off","brightness":0,"speed":0,"magic":0,"color":0}},
         "layout":{"keymap":[["KC_A","KI_LS2"],["KA_A0","SA_1","KC_LSFT","KC_NONE"],["KC_MUTE"]],"encoders":[["KC_VOLD","KC_VOLU","KC_MPLY"]],"buttons":[["KI_CS_TOGGLE"]],
                   "joystick":{"type":"RADIAL","sectors":[{"k":"KC_UP","a1":0.625,"a2":0.875},{"k":"KC_DOWN","a1":0.125,"a2":0.375},{"k":"KI_X","a1":0.875,"a2":0.125}]}}},
        {"id":1,"name":"Two","color":0,"os":0,"layout":{"keymap":[["KC_B"]],"encoders":[[{"keycode":"KC_LEFT"},{"keycode":"KC_RGHT"}]],"joystick":{"type":"JOYSTICK","sectors":[]}}}
      ]}],
      "macros":[{"id":0,"name":"cmd-c","color":0,"actions":[{"kc":"KC_LGUI","delay":0,"act":1},{"kc":"KC_C","delay":20,"act":2},{"kc":"KC_LGUI","delay":0,"act":0}]}],
      "multiActions":[{"id":3,"name":"m","kcOnTap":"KC_Q","kcOnHold":"KC_W","tt":250}]}"#;

    #[test]
    fn active_profile_layers_and_controls() {
        let doc = TWO_LAYERS.as_bytes();
        let l = parse(doc, None, 0).unwrap();
        assert_eq!((l.profile_index, l.profile_count, l.layer_count), (1, 2, 2));
        assert_eq!(
            l.keys[0],
            Binding::Key {
                mods: 0,
                code: 0x04
            }
        );
        assert_eq!(l.keys[1], Binding::LayerToggle(2));
        assert_eq!(l.keys[2], Binding::Macro(0));
        assert_eq!(l.keys[3], Binding::Smart(1));
        assert_eq!(
            l.keys[4],
            Binding::Key {
                mods: 0x02,
                code: 0
            }
        );
        assert_eq!(l.keys[5], Binding::None);
        assert_eq!(l.keys[6], Binding::Consumer(0xE2));
        assert_eq!(l.keys[7], Binding::None, "unassigned positions stay empty");
        assert_eq!(
            l.encoder,
            [
                Binding::Consumer(0xEA),
                Binding::Consumer(0xE9),
                Binding::Consumer(0xCD)
            ]
        );
        assert_eq!(l.touch, Binding::CheatSheet(2));
        assert_eq!(l.joystick, Joystick::Sectors);
        assert_eq!(l.sector_count, 3);
        assert_eq!(
            l.sectors[0],
            Sector {
                binding: Binding::Key {
                    mods: 0,
                    code: 0x52
                },
                a1: 625,
                a2: 875
            }
        );
        assert!(l.sectors[0].contains(750) && !l.sectors[0].contains(250));
        assert!(
            l.sectors[2].contains(0) && l.sectors[2].contains(900) && !l.sectors[2].contains(500),
            "wrapping sector"
        );
        // second layer, explicit profile index, object-form keycodes
        let l2 = parse(doc, Some(1), 1).unwrap();
        assert_eq!(l2.layer_index, 1);
        assert_eq!(
            l2.keys[0],
            Binding::Key {
                mods: 0,
                code: 0x05
            }
        );
        assert_eq!(
            l2.encoder[0],
            Binding::Key {
                mods: 0,
                code: 0x50
            }
        );
        assert_eq!(l2.joystick, Joystick::Sectors);
        // out-of-range layer falls back to the first
        assert_eq!(parse(doc, Some(1), 9).unwrap().layer_index, 0);
        // the other profile by index
        assert_eq!(
            parse(doc, Some(0), 0).unwrap().keys[0],
            Binding::Key {
                mods: 0,
                code: 0x1B
            }
        );
        // lights on the base layer only
        assert!(layer_lights(doc, None, 0).is_some());
        assert!(layer_lights(doc, None, 1).is_none());
    }

    #[test]
    fn macros_multis_and_smart_actions() {
        let doc = TWO_LAYERS.as_bytes();
        let mut steps = vec![];
        assert!(macro_steps(doc, 0, |s| steps.push(s)));
        assert_eq!(
            steps,
            vec![
                Step {
                    binding: Binding::Key {
                        mods: 0x08,
                        code: 0
                    },
                    delay_ms: 0,
                    act: ACT_PRESS
                },
                Step {
                    binding: Binding::Key {
                        mods: 0,
                        code: 0x06
                    },
                    delay_ms: 20,
                    act: ACT_CLICK
                },
                Step {
                    binding: Binding::Key {
                        mods: 0x08,
                        code: 0
                    },
                    delay_ms: 0,
                    act: ACT_RELEASE
                },
            ]
        );
        assert!(!macro_steps(doc, 9, |_| {}));
        assert_eq!(
            multi_tap(doc, 3),
            Some(Binding::Key {
                mods: 0,
                code: 0x14
            })
        );
        assert_eq!(multi_tap(doc, 4), None);

        let sa = br#"{"version":1,"smartActions":{"SA_0":{"name":"hi","type":"TEXT_STEP","payload":{"text":"hello"}},"SA_12":{"name":"u","type":"URL_STEP","payload":{"url":"https://x"}}},"smartActionGroups":[]}"#;
        assert_eq!(
            smart_action(sa, 0),
            Some((SmartKind::Text, br#"{"text":"hello"}"# as &[u8]))
        );
        assert_eq!(
            smart_action(sa, 12),
            Some((SmartKind::Url, br#"{"url":"https://x"}"# as &[u8]))
        );
        assert_eq!(smart_action(sa, 1), None);
        assert_eq!(SmartKind::Command.method(), b"kb.sa.exec");
    }
}

#[cfg(test)]
mod tests {
    use super::wire::*;

    const VERSION: &str = "0.8.0-openmicro";

    /// Split `json` the way a host does (61-byte payloads, report ID first,
    /// newline-terminated) and feed every fragment to `rx`, returning the
    /// last push result.
    fn feed(rx: &mut Reassembler, json: &[u8], with_report_id: bool) -> Push {
        let mut line = json.to_vec();
        line.push(b'\n');
        let mut last = Push::Pending;
        for chunk in line.chunks(PAYLOAD_MAX) {
            let mut rep = vec![];
            if with_report_id {
                rep.push(REPORT_ID);
            }
            rep.push(MSG_TYPE);
            rep.push(chunk.len() as u8);
            rep.extend_from_slice(chunk);
            rep.resize(if with_report_id { 64 } else { 63 }, 0);
            last = rx.push(&rep, &mut NoSink);
        }
        last
    }

    fn handle(req: &[u8], lights: &mut Lights) -> (Handled, String) {
        let mut store = super::MemStore::with_default();
        let mut ws = WriteState::new();
        handle_with(req, lights, &mut store, &mut ws, Status::default())
    }

    pub(super) fn handle_with(
        req: &[u8],
        lights: &mut Lights,
        store: &mut super::MemStore,
        ws: &mut WriteState,
        status: Status,
    ) -> (Handled, String) {
        let mut tx = [0u8; TX_CAP];
        let mut ctx = Ctx {
            version: VERSION,
            lights,
            store,
            write: ws,
            status,
        };
        let (what, reply) = handle_request(req, &mut ctx, &mut tx);
        let text = match reply {
            Reply::None => String::new(),
            Reply::Buf(n) => String::from_utf8(tx[..n].to_vec()).unwrap(),
            Reply::File {
                head,
                name,
                name_len,
                tail,
            } => {
                let mut v = tx[..head].to_vec();
                v.extend_from_slice(ctx.store.read(&name[..name_len as usize]).unwrap());
                v.extend_from_slice(tail);
                String::from_utf8(v).unwrap()
            }
        };
        (what, text)
    }

    fn event(ev: Event) -> String {
        let mut tx = [0u8; TX_CAP];
        let n = event_json(ev, &mut tx);
        String::from_utf8(tx[..n].to_vec()).unwrap()
    }

    // The stopwatch reference's hid_rpc_probe.swift `--demo-lights` request
    // (six agent slots, id 4243) — the shape ChatGPT Desktop is documented
    // to send.
    const THSTATUS: &[u8] = br#"{"method":"v.oai.thstatus","params":[{"id":0,"c":16777215,"b":1,"e":"off","s":0},{"id":1,"c":1754367,"b":1,"e":"breath","s":1},{"id":2,"c":4521796,"b":1,"e":"off","s":0},{"id":3,"c":16753920,"b":1,"e":"off","s":0},{"id":4,"c":16724787,"b":1,"e":"off","s":0},{"id":5,"c":0,"b":0,"e":"off","s":0}],"id":4243}"#;

    #[test]
    fn descriptor_matches_reference_bytes() {
        let expected: [u8; 29] = [
            0x06, 0x00, 0xFF, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x06, 0x15, 0x00, 0x26, 0xFF, 0x00,
            0x75, 0x08, 0x95, 0x3F, 0x09, 0x01, 0x81, 0x02, 0x95, 0x3F, 0x09, 0x02, 0x91, 0x02,
            0xC0,
        ];
        assert_eq!(REPORT_DESC, &expected);
    }

    #[test]
    fn frame_splits_into_61_byte_reports_with_newline() {
        let json = vec![b'x'; 70];
        let mut rep = [0u8; REPORT_LEN];
        assert_eq!(frame(&json, 0, &mut rep), 61);
        assert_eq!(&rep[..3], &[REPORT_ID, MSG_TYPE, 61]);
        assert!(rep[3..64].iter().all(|&c| c == b'x'));
        assert_eq!(frame(&json, 61, &mut rep), 10);
        assert_eq!(&rep[..3], &[REPORT_ID, MSG_TYPE, 10]);
        assert_eq!(&rep[3..12], &[b'x'; 9]);
        assert_eq!(rep[12], b'\n');
        assert!(rep[13..].iter().all(|&c| c == 0), "zero padded");
        assert_eq!(frame(&json, 71, &mut rep), 0);
    }

    #[test]
    fn frame_exact_fit_and_one_over() {
        let mut rep = [0u8; REPORT_LEN];
        // 60 bytes + newline = one full fragment, nothing after.
        let json = vec![b'a'; 60];
        assert_eq!(frame(&json, 0, &mut rep), 61);
        assert_eq!(rep[63], b'\n');
        assert_eq!(frame(&json, 61, &mut rep), 0);
        // 61 bytes: the newline alone spills into a second fragment.
        let json = vec![b'a'; 61];
        assert_eq!(frame(&json, 0, &mut rep), 61);
        assert_eq!(frame(&json, 61, &mut rep), 1);
        assert_eq!(&rep[..4], &[REPORT_ID, MSG_TYPE, 1, b'\n']);
    }

    #[test]
    fn reassembles_fragmented_thstatus_with_report_id() {
        let mut rx = Reassembler::new();
        let mut lights = Lights::OFF;
        let Push::Complete(len) = feed(&mut rx, THSTATUS, true) else {
            panic!("expected a complete request");
        };
        assert_eq!(rx.data(len), THSTATUS);
        let (what, reply) = handle(rx.data(len), &mut lights);
        assert_eq!(what, Handled::ThreadStatus);
        assert_eq!(reply, r#"{"id":4243,"result":{"ok":true}}"#);

        let a = &lights.agents;
        assert!(a.iter().all(|l| l.set));
        assert_eq!(
            (a[0].rgb, a[0].level, a[0].effect),
            (0xFFFFFF, 255, EFFECT_OFF)
        );
        assert_eq!(
            (a[1].rgb, a[1].level, a[1].effect),
            (0x1AC4FF, 255, EFFECT_BREATH)
        );
        assert_eq!(a[2].rgb, 0x44FF44);
        assert_eq!(a[3].rgb, 0xFFA500);
        assert_eq!(a[4].rgb, 0xFF3333);
        assert_eq!((a[5].rgb, a[5].level), (0, 0));
        assert!(!lights.keys.set && !lights.ambient.set);
    }

    #[test]
    fn reassembles_without_report_id_byte() {
        let mut rx = Reassembler::new();
        let Push::Complete(len) = feed(&mut rx, THSTATUS, false) else {
            panic!("expected a complete request");
        };
        assert_eq!(rx.data(len), THSTATUS);
    }

    #[test]
    fn partial_request_is_pending_until_closed() {
        let mut rx = Reassembler::new();
        let head = &THSTATUS[..PAYLOAD_MAX];
        let mut rep = vec![REPORT_ID, MSG_TYPE, head.len() as u8];
        rep.extend_from_slice(head);
        rep.resize(64, 0);
        assert_eq!(rx.push(&rep, &mut NoSink), Push::Pending);
    }

    #[test]
    fn fresh_method_prefix_resyncs_a_stale_partial() {
        let mut rx = Reassembler::new();
        // First fragment of a long request, never finished…
        let head = &THSTATUS[..PAYLOAD_MAX];
        let mut rep = vec![REPORT_ID, MSG_TYPE, head.len() as u8];
        rep.extend_from_slice(head);
        rep.resize(64, 0);
        assert_eq!(rx.push(&rep, &mut NoSink), Push::Pending);
        // …then a short new request in one fragment.
        let req = br#"{"method":"sys.version","id":7}"#;
        let Push::Complete(len) = feed(&mut rx, req, true) else {
            panic!("expected the new request to complete");
        };
        assert_eq!(rx.data(len), req);
    }

    #[test]
    fn leading_garbage_before_brace_is_skipped() {
        let mut rx = Reassembler::new();
        let payload = b"\n\n{\"method\":\"sys.version\",\"id\":1}";
        let mut rep = vec![REPORT_ID, MSG_TYPE, payload.len() as u8];
        rep.extend_from_slice(payload);
        rep.resize(64, 0);
        assert_eq!(
            rx.push(&rep, &mut NoSink),
            Push::Complete(payload.len() - 2)
        );
        // A newline-only fragment on an empty buffer is nothing.
        let mut rx = Reassembler::new();
        assert_eq!(
            rx.push(&[REPORT_ID, MSG_TYPE, 1, b'\n'], &mut NoSink),
            Push::Pending
        );
    }

    #[test]
    fn rejects_wrong_type_short_and_overlong_frames() {
        let mut rx = Reassembler::new();
        assert_eq!(rx.push(&[REPORT_ID], &mut NoSink), Push::Pending);
        assert_eq!(
            rx.push(&[REPORT_ID, 0x01, 3, b'{', b'}', 0], &mut NoSink),
            Push::Pending
        );
        // Length byte claims more than the packet carries.
        assert_eq!(
            rx.push(&[REPORT_ID, MSG_TYPE, 40, b'{'], &mut NoSink),
            Push::Pending
        );
        // Length byte above 61 is clamped, not trusted.
        let mut rep = vec![REPORT_ID, MSG_TYPE, 200];
        rep.extend_from_slice(br#"{"method":"sys.version","id":2}"#);
        rep.resize(64, 0);
        assert!(matches!(rx.push(&rep, &mut NoSink), Push::Complete(_)));
    }

    #[test]
    fn oversized_request_is_dropped_and_buffer_reset() {
        let mut rx = Reassembler::new();
        let mut rep = [0u8; 64];
        rep[0] = REPORT_ID;
        rep[1] = MSG_TYPE;
        rep[2] = PAYLOAD_MAX as u8;
        rep[3] = b'{';
        rep[4..64].fill(b' ');
        let mut result = Push::Pending;
        for _ in 0..(RX_CAP / PAYLOAD_MAX + 2) {
            result = rx.push(&rep, &mut NoSink);
            rep[3] = b' ';
            if result == Push::Dropped {
                break;
            }
        }
        assert_eq!(result, Push::Dropped);
        // Buffer is usable again afterwards.
        let Push::Complete(_) = feed(&mut rx, br#"{"method":"sys.version","id":3}"#, true) else {
            panic!("reassembler did not recover after a drop");
        };
    }

    #[test]
    fn device_status_reply_matches_reference_shape() {
        let mut lights = Lights::OFF;
        let mut store = super::MemStore::with_default();
        let mut ws = WriteState::new();
        let (what, reply) = handle_with(
            br#"{"method":"device.status","params":{},"id":4242}"#,
            &mut lights,
            &mut store,
            &mut ws,
            Status {
                profile_index: 0,
                layer_index: 1,
            },
        );
        assert_eq!(what, Handled::DeviceStatus);
        assert_eq!(
            reply,
            r#"{"id":4242,"result":{"version":"0.8.0-openmicro","profile_index":0,"layer_index":1,"battery":100,"is_charging":false}}"#
        );
    }

    #[test]
    fn sys_version_and_acked_methods() {
        let mut lights = Lights::OFF;
        let (what, reply) = handle(br#"{"method":"sys.version","id":1}"#, &mut lights);
        assert_eq!(what, Handled::SysVersion);
        assert_eq!(reply, r#"{"id":1,"result":{"version":"0.8.0-openmicro"}}"#);
        for (m, expect) in [
            ("lights.preview", Handled::LightsPreview),
            ("host.focused_app", Handled::Acked),
        ] {
            let req = format!(r#"{{"method":"{m}","params":{{"app":"Terminal"}},"id":9}}"#);
            let (what, reply) = handle(req.as_bytes(), &mut lights);
            assert_eq!(what, expect);
            assert_eq!(reply, r#"{"id":9,"result":{"ok":true}}"#);
        }
    }

    #[test]
    fn id_is_echoed_verbatim_or_null() {
        let mut lights = Lights::OFF;
        let (_, reply) = handle(br#"{"id":"abc-1","method":"sys.version"}"#, &mut lights);
        assert!(reply.starts_with(r#"{"id":"abc-1","#), "{reply}");
        let (_, reply) = handle(br#"{"method":"sys.version"}"#, &mut lights);
        assert!(reply.starts_with(r#"{"id":null,"#), "{reply}");
        // Key order in the request does not matter.
        let (what, reply) = handle(
            br#"{ "params" : {} , "id" : 12 , "method" : "device.status" }"#,
            &mut lights,
        );
        assert_eq!(what, Handled::DeviceStatus);
        assert!(reply.starts_with(r#"{"id":12,"#), "{reply}");
    }

    #[test]
    fn unknown_method_and_misshapen_params_get_32601() {
        let mut lights = Lights::OFF;
        let err = r#"{"id":5,"error":{"code":-32601,"message":"Method not found"}}"#;
        let (what, reply) = handle(br#"{"method":"fw.update","params":{},"id":5}"#, &mut lights);
        assert_eq!((what, reply.as_str()), (Handled::Unknown, err));
        // thstatus needs an array, rgbcfg an object — like the references.
        let (what, _) = handle(
            br#"{"method":"v.oai.thstatus","params":{},"id":5}"#,
            &mut lights,
        );
        assert_eq!(what, Handled::Unknown);
        let (what, _) = handle(
            br#"{"method":"v.oai.rgbcfg","params":[],"id":5}"#,
            &mut lights,
        );
        assert_eq!(what, Handled::Unknown);
        assert_eq!(lights, Lights::OFF);
        // No method, or not a string: still answered on the id, as the
        // references do (their missing method defaults to "").
        let (what, reply) = handle(br#"{"id":5}"#, &mut lights);
        assert_eq!((what, reply.as_str()), (Handled::Unknown, err));
        let (what, reply) = handle(br#"{"method":7,"id":5}"#, &mut lights);
        assert_eq!((what, reply.as_str()), (Handled::Unknown, err));
    }

    #[test]
    fn thstatus_fields_are_optional_and_sticky() {
        let mut lights = Lights::OFF;
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"c":255,"b":0.5,"e":"off","s":0}],"id":1}"#,
            &mut lights,
        );
        assert_eq!(
            (lights.agents[2].rgb, lights.agents[2].level),
            (0x0000FF, 127)
        );
        assert_eq!(lights.agents[2].effect, EFFECT_OFF);
        // Only the effect and speed change: colour and level stick.
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"e":4,"s":0.4}],"id":2}"#,
            &mut lights,
        );
        assert_eq!(
            (lights.agents[2].rgb, lights.agents[2].level),
            (0x0000FF, 127)
        );
        assert_eq!(
            (lights.agents[2].effect, lights.agents[2].speed()),
            (EFFECT_BREATH, 400)
        );
        // Out-of-range slot ignored, others untouched; b clamps; names work.
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":9,"c":1},{"id":0,"c":16711680,"b":7,"e":"breath","s":0}],"id":3}"#,
            &mut lights,
        );
        assert_eq!(
            (lights.agents[0].rgb, lights.agents[0].level),
            (0xFF0000, 255)
        );
        assert_eq!(lights.agents[0].effect, EFFECT_BREATH);
        assert!(!lights.agents[1].set);
    }

    #[test]
    fn rgbcfg_sets_ambient_and_keys_independently() {
        let mut lights = Lights::OFF;
        let (what, reply) = handle(
            br#"{"method":"v.oai.rgbcfg","params":{"ambient":{"c":65280,"b":0.25,"e":"off","s":0}},"id":8}"#,
            &mut lights,
        );
        assert_eq!(what, Handled::RgbConfig);
        assert_eq!(reply, r#"{"id":8,"result":{"ok":true}}"#);
        assert!(lights.ambient.set && !lights.keys.set);
        assert_eq!((lights.ambient.rgb, lights.ambient.level), (0x00FF00, 63));
        handle(
            br#"{"method":"v.oai.rgbcfg","params":{"keys":{"c":16777215,"b":1,"e":"breath","s":1},"ambient":null},"id":9}"#,
            &mut lights,
        );
        assert!(lights.keys.set && lights.keys.effect == EFFECT_BREATH);
        assert_eq!(lights.ambient.level, 63, "null side leaves the old value");
    }

    /// The exact shapes the Codex desktop app's device kit emits: numeric
    /// effects (off 0, solid 1, snake 2, rainbow 3, breath 4, gradient 5,
    /// shallowBreath 6), sync flags on threads, `m` (magic) on sides.
    #[test]
    fn real_app_numeric_effects_and_extra_fields() {
        let mut lights = Lights::OFF;
        let (what, _) = handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":0,"c":0,"b":0,"e":0,"s":0,"sk":0,"sa":0},{"id":1,"c":1754367,"b":1,"e":4,"s":0.4,"sk":0,"sa":0},{"id":2,"c":4521796,"b":1,"e":1,"s":0,"sk":0,"sa":0}],"id":7}"#,
            &mut lights,
        );
        assert_eq!(what, Handled::ThreadStatus);
        assert_eq!(
            (lights.agents[0].effect, lights.agents[0].level),
            (EFFECT_OFF, 0)
        );
        assert_eq!(
            (lights.agents[1].effect, lights.agents[1].speed()),
            (EFFECT_BREATH, 400)
        );
        assert_eq!(
            (lights.agents[2].effect, lights.agents[2].speed()),
            (EFFECT_SOLID, 0)
        );
        let (what, _) = handle(
            br#"{"method":"v.oai.rgbcfg","params":{"ambient":{"e":2,"b":1,"s":0.4,"m":0,"c":3050327},"keys":{"e":0,"b":0,"s":0,"m":0,"c":0}},"id":8}"#,
            &mut lights,
        );
        assert_eq!(what, Handled::RgbConfig);
        assert_eq!(
            (
                lights.ambient.effect,
                lights.ambient.rgb,
                lights.ambient.speed()
            ),
            (EFFECT_SNAKE, 0x2E8B57, 400)
        );
        assert_eq!((lights.keys.effect, lights.keys.level), (EFFECT_OFF, 0));
        // An effect number we do not know is kept (the renderer treats it
        // as solid); a name we do not know leaves the effect alone.
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"e":9}],"id":9}"#,
            &mut lights,
        );
        assert_eq!(lights.agents[2].effect, 9);
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"e":"sparkle"}],"id":10}"#,
            &mut lights,
        );
        assert_eq!(lights.agents[2].effect, 9);
        // Negative or oversized speeds clamp.
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"s":-1}],"id":11}"#,
            &mut lights,
        );
        assert_eq!(lights.agents[2].speed(), 0);
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"s":7}],"id":12}"#,
            &mut lights,
        );
        assert_eq!(lights.agents[2].speed(), 1000);
    }

    #[test]
    fn events_match_reference_json() {
        assert_eq!(
            event(Event::Key {
                key: Key::Position(2),
                act: ACT_PRESS
            }),
            r#"{"method":"v.oai.hid","params":{"k":"AG02","act":1,"ag":2}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::Position(5),
                act: ACT_RELEASE
            }),
            r#"{"method":"v.oai.hid","params":{"k":"AG05","act":0,"ag":5}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::Position(6),
                act: ACT_PRESS
            }),
            r#"{"method":"v.oai.hid","params":{"k":"ACT06","act":1}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::Position(10),
                act: ACT_PRESS
            }),
            r#"{"method":"v.oai.hid","params":{"k":"ACT10","act":1}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::Position(12),
                act: ACT_RELEASE
            }),
            r#"{"method":"v.oai.hid","params":{"k":"ACT12","act":0}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::EncCw,
                act: ACT_STEP
            }),
            r#"{"method":"v.oai.hid","params":{"k":"ENC_CW","act":2}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::EncCcw,
                act: ACT_STEP
            }),
            r#"{"method":"v.oai.hid","params":{"k":"ENC_CC","act":2}}"#
        );
        assert_eq!(
            event(Event::Key {
                key: Key::EncPress,
                act: ACT_PRESS
            }),
            r#"{"method":"v.oai.hid","params":{"k":"ENC","act":1}}"#
        );
        assert_eq!(
            event(Event::Stick {
                dir: 0,
                pressed: true
            }),
            r#"{"method":"v.oai.rad","params":{"a":0.75,"d":1}}"#
        );
        assert_eq!(
            event(Event::Stick {
                dir: 1,
                pressed: false
            }),
            r#"{"method":"v.oai.rad","params":{"a":0.25,"d":0}}"#
        );
        assert_eq!(
            event(Event::Stick {
                dir: 2,
                pressed: true
            }),
            r#"{"method":"v.oai.rad","params":{"a":0.5,"d":1}}"#
        );
        assert_eq!(
            event(Event::Stick {
                dir: 3,
                pressed: true
            }),
            r#"{"method":"v.oai.rad","params":{"a":0,"d":1}}"#
        );
    }

    #[test]
    fn every_event_fits_one_fragment() {
        // Keeps a keypress to a single report so it can never be torn.
        for p in 0..13u8 {
            assert!(
                event(Event::Key {
                    key: Key::Position(p),
                    act: ACT_PRESS
                })
                .len()
                    < PAYLOAD_MAX
            );
        }
    }

    #[test]
    fn reply_that_would_overflow_tx_is_suppressed() {
        let mut lights = Lights::OFF;
        let mut store = super::MemStore::with_default();
        let mut ws = WriteState::new();
        let mut ctx = Ctx {
            version: VERSION,
            lights: &mut lights,
            store: &mut store,
            write: &mut ws,
            status: Status::default(),
        };
        let mut tx = [0u8; 24];
        let (_, reply) = handle_request(br#"{"method":"sys.version","id":1}"#, &mut ctx, &mut tx);
        assert_eq!(reply, Reply::None);
    }
}

/// In-memory `FileStore` mirroring the flash store's semantics: two named
/// slots, the built-in keymap when the slot is empty, SHA-1 in the listing.
#[derive(Default)]
pub struct MemStore {
    files: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    writing: Option<(Vec<u8>, Vec<u8>, bool)>,
}

impl MemStore {
    pub fn with_default() -> Self {
        Self::default()
    }
    fn slot(name: &[u8]) -> Option<&'static [u8]> {
        match name.strip_prefix(b"/").unwrap_or(name) {
            b"keymap.json" => Some(b"keymap.json"),
            b"smart_actions.json" => Some(b"smart_actions.json"),
            _ => None,
        }
    }
}

impl wire::FileStore for MemStore {
    fn each_file(&mut self, f: &mut dyn FnMut(&[u8], usize, &[u8; 20])) {
        for name in [b"keymap.json" as &[u8], b"smart_actions.json"] {
            if let Some(body) = self.read(name) {
                f(name, body.len(), &sha1::digest(body));
            }
        }
    }
    fn read(&self, name: &[u8]) -> Option<&'static [u8]> {
        let name = Self::slot(name)?;
        match self.files.get(name) {
            Some(v) => Some(Box::leak(v.clone().into_boxed_slice())),
            None if name == b"keymap.json" => Some(layout::DEFAULT_KEYMAP.as_bytes()),
            None => None,
        }
    }
    fn begin_write(&mut self, name: &[u8]) -> bool {
        match Self::slot(name) {
            Some(n) => {
                self.writing = Some((n.to_vec(), Vec::new(), true));
                true
            }
            None => false,
        }
    }
    fn write(&mut self, data: &[u8]) -> bool {
        match self.writing.as_mut() {
            Some((_, buf, ok)) => {
                if buf.len() + data.len() > 12 * 1024 - 32 {
                    *ok = false;
                    return false;
                }
                buf.extend_from_slice(data);
                *ok
            }
            None => false,
        }
    }
    fn finish_write(&mut self) -> bool {
        match self.writing.take() {
            Some((name, buf, true)) => {
                self.files.insert(name, buf);
                true
            }
            _ => false,
        }
    }
    fn abort_write(&mut self) {
        self.writing = None;
    }
    fn delete(&mut self, name: &[u8]) -> bool {
        match Self::slot(name) {
            Some(n) => {
                self.files.remove(n);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod fs_tests {
    use super::tests::handle_with;
    use super::wire::*;
    use super::{layout, sha1, MemStore};

    fn hexs(d: &[u8; 20]) -> String {
        let mut h = [0u8; 40];
        sha1::hex(d, &mut h);
        String::from_utf8(h.to_vec()).unwrap()
    }

    fn rpc(store: &mut MemStore, ws: &mut WriteState, req: &str) -> String {
        let mut lights = Lights::OFF;
        handle_with(req.as_bytes(), &mut lights, store, ws, Status::default()).1
    }

    #[test]
    fn deleting_a_file_asks_for_a_reload() {
        let mut store = MemStore::default();
        let mut ws = WriteState::new();
        let mut lights = Lights::default();
        let (what, reply) = handle_with(
            br#"{"method":"fs.delete","params":{"file":"keymap.json"},"id":9}"#,
            &mut lights,
            &mut store,
            &mut ws,
            Status::default(),
        );
        assert_eq!(reply, r#"{"id":9,"result":{"ok":true}}"#);
        assert_eq!(what, Handled::FileWritten);
        let (what, reply) = handle_with(
            br#"{"method":"fs.delete","params":{"file":"nope.json"},"id":10}"#,
            &mut lights,
            &mut store,
            &mut ws,
            Status::default(),
        );
        assert!(reply.contains("File does not exist"), "{reply}");
        assert_eq!(what, Handled::File);
    }

    #[test]
    fn list_serves_the_builtin_keymap_with_its_sha1() {
        let mut store = MemStore::with_default();
        let mut ws = WriteState::new();
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.list","params":{"checksum":true},"id":317}"#,
        );
        let sha = hexs(&sha1::digest(layout::DEFAULT_KEYMAP.as_bytes()));
        assert_eq!(
            reply,
            format!(
                r#"{{"id":317,"result":[{{"name":"keymap.json","size":"{}","checksum":"{}"}}]}}"#,
                layout::DEFAULT_KEYMAP.len(),
                sha
            )
        );
    }

    #[test]
    fn readbin_chunks_reassemble_the_file() {
        let mut store = MemStore::with_default();
        let mut ws = WriteState::new();
        let file = layout::DEFAULT_KEYMAP.as_bytes();
        let mut got = Vec::new();
        let mut offset = 0;
        loop {
            let req = format!(
                r#"{{"method":"fs.readbin","params":{{"file":"keymap.json","offset":{offset},"len":3072}},"id":9}}"#
            );
            let reply = rpc(&mut store, &mut ws, &req);
            let result = find_key(reply.as_bytes(), b"result").unwrap();
            let total = parse_u32(find_key(result, b"total_size").unwrap()).unwrap();
            assert_eq!(total as usize, file.len());
            let data = as_str(find_key(result, b"data").unwrap()).unwrap();
            let mut dec = B64Decode::new();
            let mut chunk = Vec::new();
            assert!(dec.feed(data, &mut |b| chunk.extend_from_slice(b)));
            dec.finish(&mut |b| chunk.extend_from_slice(b));
            assert!(!chunk.is_empty() && chunk.len() <= READ_CHUNK);
            got.extend_from_slice(&chunk);
            offset += chunk.len();
            if offset >= file.len() {
                break;
            }
        }
        assert_eq!(got, file);
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.readbin","params":{"file":"nope.json","offset":0,"len":10},"id":1}"#,
        );
        assert_eq!(
            reply,
            r#"{"id":1,"error":{"code":-2,"message":"File does not exist"}}"#
        );
    }

    /// The Input app's writer: 3072-byte raw chunks as 4096 base64 chars,
    /// each its own fs.writebin request, all split into 61-byte reports.
    #[test]
    fn streamed_writebin_stores_the_file_without_buffering_it() {
        let mut store = MemStore::with_default();
        let mut ws = WriteState::new();
        let mut rx = Reassembler::new();
        let pattern = b"{\"k\":1}";
        let body: Vec<u8> = (0..7000usize).map(|i| pattern[i % pattern.len()]).collect();
        let b64 = |d: &[u8]| {
            let mut tx = vec![0u8; d.len() * 2 + 8];
            let mut o = Out::new(&mut tx);
            o.push_base64(d);
            let n = o.finish();
            String::from_utf8(tx[..n].to_vec()).unwrap()
        };
        let chunks: Vec<&[u8]> = body.chunks(3072).collect();
        let mut replies = vec![];
        for (i, chunk) in chunks.iter().enumerate() {
            let req = format!(
                r#"{{"method":"fs.writebin","params":{{"file":"keymap.json","data":"{}","append":true,"completed":{},"offset":{}}},"id":{}}}"#,
                b64(chunk),
                i == chunks.len() - 1,
                i * 3072,
                100 + i
            );
            let mut line = req.into_bytes();
            line.push(b'\n');
            let mut result = Push::Pending;
            for frag in line.chunks(PAYLOAD_MAX) {
                let mut rep = vec![REPORT_ID, MSG_TYPE, frag.len() as u8];
                rep.extend_from_slice(frag);
                rep.resize(64, 0);
                let mut sink = WriteSink {
                    state: &mut ws,
                    store: &mut store,
                };
                result = rx.push(&rep, &mut sink);
            }
            let Push::Complete(len) = result else {
                panic!("chunk {i} did not complete: {result:?}");
            };
            let mut lights = Lights::OFF;
            let (what, reply) = handle_with(
                rx.data(len),
                &mut lights,
                &mut store,
                &mut ws,
                Status::default(),
            );
            rx.clear();
            replies.push(reply);
            assert_eq!(
                what,
                if i == chunks.len() - 1 {
                    Handled::FileWritten
                } else {
                    Handled::File
                }
            );
        }
        assert_eq!(replies[0], r#"{"id":100,"result":{"data_written":3072}}"#);
        assert_eq!(replies[2], r#"{"id":102,"result":{"data_written":856}}"#);
        assert_eq!(store.read(b"keymap.json").unwrap(), &body[..]);
        // and the listing now carries the new size + sha1
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.list","params":{"checksum":true},"id":2}"#,
        );
        assert!(
            reply.contains(&format!(
                r#""size":"{}","checksum":"{}""#,
                body.len(),
                hexs(&sha1::digest(&body))
            )),
            "{reply}"
        );
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let mut store = MemStore::with_default();
        let mut ws = WriteState::new();
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.write","params":{"file":"smart_actions.json","data":{"version":1,"smartActions":{"SA_0":{"type":"URL_STEP","payload":{"url":"https://a"}}}}},"id":3}"#,
        );
        assert_eq!(reply, r#"{"id":3,"result":{"ok":true}}"#);
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.read","params":{"file":"smart_actions.json"},"id":4}"#,
        );
        assert_eq!(
            reply,
            r#"{"id":4,"result":{"version":1,"smartActions":{"SA_0":{"type":"URL_STEP","payload":{"url":"https://a"}}}}}"#
        );
        assert_eq!(
            layout::smart_action(store.read(b"smart_actions.json").unwrap(), 0).map(|(k, _)| k),
            Some(layout::SmartKind::Url)
        );
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.list","params":{"checksum":true},"id":5}"#,
        );
        assert!(reply.contains(r#""name":"smart_actions.json""#));
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.delete","params":{"file":"smart_actions.json"},"id":6}"#,
        );
        assert_eq!(reply, r#"{"id":6,"result":{"ok":true}}"#);
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.read","params":{"file":"smart_actions.json"},"id":7}"#,
        );
        assert_eq!(
            reply,
            r#"{"id":7,"error":{"code":-2,"message":"File does not exist"}}"#
        );
        let reply = rpc(
            &mut store,
            &mut ws,
            r#"{"method":"fs.write","params":{"file":"other.bin","data":"x"},"id":8}"#,
        );
        assert!(reply.contains("write failed"));
        assert_eq!(
            rpc(&mut store, &mut ws, r#"{"method":"fs.txbegin","id":9}"#),
            r#"{"id":9,"result":{"tx":1}}"#
        );
        assert_eq!(
            rpc(
                &mut store,
                &mut ws,
                r#"{"method":"fs.txcommit","params":{"tx":1},"id":10}"#
            ),
            r#"{"id":10,"result":{"ok":true}}"#
        );
        assert_eq!(
            rpc(
                &mut store,
                &mut ws,
                r#"{"method":"fs.rmdir","params":{"path":"/apps"},"id":11}"#
            ),
            r#"{"id":11,"result":{"ok":true}}"#
        );
    }

    #[test]
    fn input_app_misc_methods_and_preview() {
        let mut store = MemStore::with_default();
        let mut ws = WriteState::new();
        assert_eq!(
            rpc(
                &mut store,
                &mut ws,
                r#"{"method":"ui.active_screen","id":1}"#
            ),
            r#"{"id":1,"result":{"screen_name":"home"}}"#
        );
        assert_eq!(
            rpc(
                &mut store,
                &mut ws,
                r#"{"method":"appmgr.list_installed","id":2}"#
            ),
            r#"{"id":2,"result":[]}"#
        );
        assert_eq!(
            rpc(&mut store, &mut ws, r#"{"method":"sys.selftest","id":3}"#),
            r#"{"id":3,"result":{"ok":true}}"#
        );
        let mut lights = Lights::OFF;
        let (what, reply) = handle_with(
            br#"{"method":"lights.preview","params":{"backlight":{"effect":"breath","brightness":0.5,"speed":0.4,"magic":0,"color":16711680},"underglow":{"effect":"solid","brightness":1,"speed":0,"magic":0,"color":255}},"id":4}"#,
            &mut lights, &mut store, &mut ws, Status::default(),
        );
        assert_eq!(
            (what, reply.as_str()),
            (Handled::LightsPreview, r#"{"id":4,"result":{"ok":true}}"#)
        );
        assert_eq!(
            (
                lights.keys.effect,
                lights.keys.level,
                lights.keys.rgb,
                lights.keys.speed()
            ),
            (EFFECT_BREATH, 127, 0xFF0000, 400)
        );
        assert_eq!(
            (
                lights.ambient.effect,
                lights.ambient.level,
                lights.ambient.rgb
            ),
            (EFFECT_SOLID, 255, 0x0000FF)
        );
    }

    #[test]
    fn notifications_for_the_input_app() {
        let mut tx = [0u8; TX_CAP];
        let n = event_json(
            Event::CheatSheet {
                mode: 1,
                layer: 0,
                profile: 2,
            },
            &mut tx,
        );
        assert_eq!(
            std::str::from_utf8(&tx[..n]).unwrap(),
            r#"{"method":"kb.cs.show","params":{"l":0,"p":2}}"#
        );
        let n = event_json(
            Event::Radial {
                angle_milli: 750,
                open: true,
                layer: 1,
                profile: 0,
            },
            &mut tx,
        );
        assert_eq!(
            std::str::from_utf8(&tx[..n]).unwrap(),
            r#"{"method":"kb.radial","params":{"a":0.75,"d":1,"l":1,"p":0,"o":1}}"#
        );
        let n = event_json(
            Event::Radial {
                angle_milli: 0,
                open: false,
                layer: 0,
                profile: 0,
            },
            &mut tx,
        );
        assert_eq!(
            std::str::from_utf8(&tx[..n]).unwrap(),
            r#"{"method":"kb.radial","params":{"a":0,"d":0,"l":0,"p":0,"o":0}}"#
        );
        let n = notify_head(b"kb.sa.openurl", &mut tx);
        assert_eq!(
            std::str::from_utf8(&tx[..n]).unwrap(),
            r#"{"method":"kb.sa.openurl","params":"#
        );
    }

    #[test]
    fn base64_roundtrip_and_milli_formatting() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len as u32).map(|i| (i * 37 + 11) as u8).collect();
            let mut buf = [0u8; 128];
            let mut o = Out::new(&mut buf);
            o.push_base64(&data);
            let n = o.finish();
            let mut dec = B64Decode::new();
            let mut back = Vec::new();
            assert!(dec.feed(&buf[..n], &mut |b| back.extend_from_slice(b)));
            dec.finish(&mut |b| back.extend_from_slice(b));
            assert_eq!(back, data, "len {len}");
        }
        let mut dec = B64Decode::new();
        assert!(!dec.feed(b"ab$c", &mut |_| {}));
        let mut buf = [0u8; 32];
        for (m, want) in [
            (0u16, "0"),
            (250, "0.25"),
            (500, "0.5"),
            (750, "0.75"),
            (1000, "1"),
            (125, "0.125"),
            (1, "0.001"),
        ] {
            let mut o = Out::new(&mut buf);
            o.push_milli(m);
            let n = o.finish();
            assert_eq!(std::str::from_utf8(&buf[..n]).unwrap(), want);
        }
    }
}
