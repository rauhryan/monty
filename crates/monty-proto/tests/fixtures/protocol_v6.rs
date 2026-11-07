//! Historical v6 negotiation negative control, extracted from monty-proto/src/lib.rs
//! at 9d7840abd7df6df9ac4e7ce6ec61dadfcc496c78. Keep this fixture at version 6.

use std::ops::RangeInclusive;

const PROTOCOL_VERSION: u32 = 6;
const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 5;
const SUPPORTED_PROTOCOL_VERSIONS: RangeInclusive<u32> = MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION;

pub fn check_protocol_version(version: u32) -> Result<(), String> {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
        Ok(())
    } else {
        // Singular while the window is one version wide — "2 to 2" reads as a range it is not.
        // `let`-bound so the `format_args!` temporaries live long enough to be formatted below.
        // use "server" since this error should only ever happen when using a client/server architecture
        let supported = if MIN_SUPPORTED_PROTOCOL_VERSION == PROTOCOL_VERSION {
            format_args!("server supports protocol version {PROTOCOL_VERSION}")
        } else {
            format_args!("server supports protocol versions {MIN_SUPPORTED_PROTOCOL_VERSION} to {PROTOCOL_VERSION}")
        };

        // Points the reader at the client for the refusal above: the worker is typically a deployed build the
        // reader cannot change, so the client is the half to move.
        let tip = if version < MIN_SUPPORTED_PROTOCOL_VERSION {
            "try updating to a newer client version"
        } else {
            "make sure you are using the correct client version"
        };

        Err(format!("unsupported protocol version {version} ({supported}, {tip})"))
    }
}
