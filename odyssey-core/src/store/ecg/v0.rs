use std::fmt::Debug;

use crate::store::ecg::ECGHeader;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct HeaderId<Hash>(Hash);

// TODO: Move this to the right location.
/// An ECG header.
#[derive(Clone)]
pub struct Header<Hash> {
    /// The header ids of our parents in the ECG graph.
    parent_ids: Vec<HeaderId<Hash>>,

    // /// The hash of (batched) operations in this header.
    // /// The maximum number of operations is 256.
    // operation_hashes: Vec<Hash>,
}

// TODO: OperationID's are header ids and index (HeaderId, u8)

impl<Hash: Clone + Copy + Debug + Ord> ECGHeader for Header<Hash> {
    type HeaderId = HeaderId<Hash>;

    fn get_parent_ids(&self) -> &[HeaderId<Hash>] {
        &self.parent_ids
    }

    fn get_header_id(&self) -> HeaderId<Hash> {
        todo!()
    }

    fn validate_header(&self, header_id: HeaderId<Hash>) -> bool {
        // TODO: Actually check this.
        true
    }
}

#[derive(Clone, Debug)]
pub struct TestHeader {
    header_id: u32,
    parent_ids: Vec<u32>,
}

// For testing, just have the header store the parent ids.
impl ECGHeader for TestHeader {
    type HeaderId = u32;

    fn get_parent_ids(&self) -> &[u32] {
        &self.parent_ids
    }

    fn get_header_id(&self) -> u32 {
        self.header_id
    }

    fn validate_header(&self, header_id: Self::HeaderId) -> bool {
        true
    }
}

