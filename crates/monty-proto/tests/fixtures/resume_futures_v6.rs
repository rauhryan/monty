//! Historical generated ResumeFutures from 9d7840abd7df6df9ac4e7ce6ec61dadfcc496c78.
//! Standard Vec/Option paths shortened for repository lint; fields/tags unchanged.
//! Negative control only: the canonical current oracle lives in tests/oracle.

use crate::oracle::{Arena, FutureResult};

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ResumeFutures {
    /// Also answers an eager FunctionCall with exactly one result matching its
    /// call_id. The worker creates a settled awaitable before continuing.
    #[prost(message, repeated, tag = "1")]
    pub results: Vec<FutureResult>,
    /// The arena every `return_value` indexes.
    #[prost(message, optional, tag = "2")]
    pub values: Option<Arena>,
}
