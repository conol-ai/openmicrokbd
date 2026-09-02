//! Compiles the firmware's Codex Micro codec (`fw/src/codex/wire.rs`, pure
//! `core`) for the host and exercises it against the request shapes the
//! reference projects' host probes send (their protocol notes are the spec). Run via
//! `scripts/test-codex-wire.sh`.

#[path = "../../src/codex/wire.rs"]
#[allow(dead_code)]
pub mod wire;

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
            last = rx.push(&rep);
        }
        last
    }

    fn handle(req: &[u8], lights: &mut Lights) -> (Handled, String) {
        let mut tx = [0u8; TX_CAP];
        let (what, n) = handle_request(req, VERSION, lights, &mut tx);
        (what, String::from_utf8(tx[..n].to_vec()).unwrap())
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
            (a[0].rgb, a[0].level, a[0].breath()),
            (0xFFFFFF, 255, false)
        );
        assert_eq!((a[1].rgb, a[1].level, a[1].breath()), (0x1AC4FF, 255, true));
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
        assert_eq!(rx.push(&rep), Push::Pending);
    }

    #[test]
    fn fresh_method_prefix_resyncs_a_stale_partial() {
        let mut rx = Reassembler::new();
        // First fragment of a long request, never finished…
        let head = &THSTATUS[..PAYLOAD_MAX];
        let mut rep = vec![REPORT_ID, MSG_TYPE, head.len() as u8];
        rep.extend_from_slice(head);
        rep.resize(64, 0);
        assert_eq!(rx.push(&rep), Push::Pending);
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
        assert_eq!(rx.push(&rep), Push::Complete(payload.len() - 2));
        // A newline-only fragment on an empty buffer is nothing.
        let mut rx = Reassembler::new();
        assert_eq!(rx.push(&[REPORT_ID, MSG_TYPE, 1, b'\n']), Push::Pending);
    }

    #[test]
    fn rejects_wrong_type_short_and_overlong_frames() {
        let mut rx = Reassembler::new();
        assert_eq!(rx.push(&[REPORT_ID]), Push::Pending);
        assert_eq!(rx.push(&[REPORT_ID, 0x01, 3, b'{', b'}', 0]), Push::Pending);
        // Length byte claims more than the packet carries.
        assert_eq!(rx.push(&[REPORT_ID, MSG_TYPE, 40, b'{']), Push::Pending);
        // Length byte above 61 is clamped, not trusted.
        let mut rep = vec![REPORT_ID, MSG_TYPE, 200];
        rep.extend_from_slice(br#"{"method":"sys.version","id":2}"#);
        rep.resize(64, 0);
        assert!(matches!(rx.push(&rep), Push::Complete(_)));
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
            result = rx.push(&rep);
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
        let (what, reply) = handle(
            br#"{"method":"device.status","params":{},"id":4242}"#,
            &mut lights,
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
        for m in ["lights.preview", "host.focused_app"] {
            let req = format!(r#"{{"method":"{m}","params":{{"app":"Terminal"}},"id":9}}"#);
            let (what, reply) = handle(req.as_bytes(), &mut lights);
            assert_eq!(what, Handled::Acked);
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
        assert!(!lights.agents[2].breath());
        // Only the speed changes: colour and level stick, focus via s>0.01.
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":2,"s":0.4}],"id":2}"#,
            &mut lights,
        );
        assert_eq!(
            (lights.agents[2].rgb, lights.agents[2].level),
            (0x0000FF, 127)
        );
        assert!(lights.agents[2].breath());
        // Out-of-range slot ignored, others untouched; b clamps.
        handle(
            br#"{"method":"v.oai.thstatus","params":[{"id":9,"c":1},{"id":0,"c":16711680,"b":7,"e":"breath","s":0}],"id":3}"#,
            &mut lights,
        );
        assert_eq!(
            (lights.agents[0].rgb, lights.agents[0].level),
            (0xFF0000, 255)
        );
        assert!(lights.agents[0].breath());
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
        assert!(lights.keys.set && lights.keys.breath());
        assert_eq!(lights.ambient.level, 63, "null side leaves the old value");
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
        let mut tx = [0u8; 24];
        let (_, n) = handle_request(
            br#"{"method":"sys.version","id":1}"#,
            VERSION,
            &mut lights,
            &mut tx,
        );
        assert_eq!(n, 0);
    }
}
