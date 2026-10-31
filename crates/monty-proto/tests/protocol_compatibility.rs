//! Execute the historical worker's version check against current Configure bytes.

use monty_proto::{PROTOCOL_VERSION, check_protocol_version, decode_frame, pb};
use prost::Message;

#[path = "fixtures/protocol_v6.rs"]
mod protocol_v6;

#[test]
fn cancellation_parent_is_refused_by_v6_worker_check() {
    let request = pb::Configure {
        protocol_version: PROTOCOL_VERSION,
        ..Default::default()
    };
    let decoded: pb::Configure = decode_frame(&request.encode_to_vec()).unwrap();
    assert!(decoded.protocol_version >= 7);
    assert!(
        protocol_v6::check_protocol_version(decoded.protocol_version)
            .unwrap_err()
            .contains("server supports protocol versions 5 to 6")
    );
    // Negative control: the pre-repair declaration was accepted and offered no protection.
    assert!(protocol_v6::check_protocol_version(6).is_ok());
    assert!(check_protocol_version(decoded.protocol_version).is_ok());
}

#[test]
fn old_clients_are_rejected_at_cancellation_type_boundary() {
    for version in [5, 6] {
        assert!(protocol_v6::check_protocol_version(version).is_ok());
        assert!(check_protocol_version(version).is_err());
    }
    for version in [0, 4, 7, 8, PROTOCOL_VERSION + 1] {
        assert!(check_protocol_version(version).is_err());
    }
}
