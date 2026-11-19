use crate::error::{classify_retcode, Mt5Error, RetcodeClass};
use crate::types::{OrderState, RejectionEvidence};

/// Classify submission outcome to ensure only verifiably unplaced orders transition to Rejected.
pub(crate) fn classify_send_error(
    err: &Mt5Error,
) -> (OrderState, Option<RejectionEvidence>, String) {
    match err {
        Mt5Error::TransmissionFailed(msg) => (
            OrderState::Rejected,
            Some(RejectionEvidence::NeverTransmitted),
            msg.clone(),
        ),
        Mt5Error::PreflightRejected(msg) => (
            OrderState::Rejected,
            Some(RejectionEvidence::PreflightRejected),
            msg.clone(),
        ),
        Mt5Error::OrderSendFailed {
            retcode,
            description,
            ..
        } => match classify_retcode(*retcode) {
            RetcodeClass::DefinitivelyNotExecuted => (
                OrderState::Rejected,
                Some(RejectionEvidence::BrokerRetcode(*retcode as i32)),
                format!("retcode {retcode}: {description}"),
            ),
            RetcodeClass::Ambiguous | RetcodeClass::Executed => (
                OrderState::Unknown,
                None,
                format!("ambiguous retcode {retcode}: {description}"),
            ),
        },
        Mt5Error::UnknownExecutionState { description, .. } => {
            (OrderState::Unknown, None, description.clone())
        }
        other => (
            OrderState::Unknown,
            None,
            format!("unverified transmission error: {other}"),
        ),
    }
}
