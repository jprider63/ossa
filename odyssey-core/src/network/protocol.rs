
pub enum ProtocolVersion {
    V0,
}

pub(crate) fn run_handshake_server<Transport>(stream: &Transport) -> ProtocolVersion {
    // TODO: Implement this and make it abstract.

    ProtocolVersion::V0
}
