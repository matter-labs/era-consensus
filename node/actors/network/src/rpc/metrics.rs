//! Metrics for RPCs.

use super::Rpc;
use std::{any::Any, time::Duration};
use vise::{
    Buckets, EncodeLabelSet, EncodeLabelValue, Family, Gauge, Histogram, LabeledFamily, Metrics,
    Unit,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
pub(super) enum CallLatencyType {
    ClientSendRecv,
    ServerRecvSend,
    ServerProcess,
}

impl CallLatencyType {
    pub(super) fn to_labels<R: Rpc>(
        self,
        req: &R::Req,
        result: &anyhow::Result<impl Any>,
    ) -> CallLatencyLabels {
        CallLatencyLabels {
            r#type: self,
            method: R::METHOD,
            submethod: R::submethod(req),
            result: match result {
                Ok(_) => ResultLabel::Ok,
                Err(_) => ResultLabel::Err,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
enum ResultLabel {
    Ok,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet)]
pub(super) struct CallLatencyLabels {
    r#type: CallLatencyType,
    method: &'static str,
    submethod: &'static str,
    result: ResultLabel,
}

impl CallLatencyLabels {
    pub(super) fn set_result(&mut self, result: &anyhow::Result<impl Any>) {
        self.result = match result {
            Ok(_) => ResultLabel::Ok,
            Err(_) => ResultLabel::Err,
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
pub(super) enum CallType {
    Client,
    Server,
    ReqSent,
    ReqRecv,
    RespSent,
    RespRecv,
}

impl CallType {
    pub(super) fn to_labels<R: Rpc>(self, req: &R::Req) -> CallLabels {
        CallLabels {
            r#type: self,
            method: R::METHOD,
            submethod: R::submethod(req),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet)]
pub(super) struct CallLabels {
    r#type: CallType,
    method: &'static str,
    submethod: &'static str,
}

const MESSAGE_SIZE_BUCKETS: Buckets =
    Buckets::exponential(protobuf_utils::kB as f64..=protobuf_utils::MB as f64, 2.0);

#[derive(Debug, Metrics)]
#[metrics(prefix = "network_rpc")]
pub(super) struct RpcMetrics {
    /// Latency of RPCs in seconds.
    #[metrics(unit = Unit::Seconds, buckets = Buckets::LATENCIES)]
    pub(super) latency: Family<CallLatencyLabels, Histogram<Duration>>,
    /// Current number of executing RPCs.
    pub(super) inflight: Family<CallLabels, Gauge>,
    /// RPC message sizes in bytes.
    #[metrics(unit = Unit::Bytes, buckets = MESSAGE_SIZE_BUCKETS)]
    pub(super) message_size: Family<CallLabels, Histogram<usize>>,
    /// Time that client waits for the server to prepare a stream for an RPC call.
    #[metrics(unit = Unit::Seconds, buckets = Buckets::LATENCIES, labels = ["method"])]
    pub(super) call_reserve_latency: LabeledFamily<&'static str, Histogram<Duration>>,
}

#[vise::register]
pub(super) static RPC_METRICS: vise::Global<RpcMetrics> = vise::Global::new();
