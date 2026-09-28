//! AUDITOR_NEGATIVE_FIXTURE — fail-closed unwired transport enum is legitimate.
pub enum TransportError {
    InertUnavailable,
}
pub struct InertTransport;
impl InertTransport {
    pub fn submit(&self) -> Result<(), TransportError> {
        Err(TransportError::InertUnavailable)
    }
}
