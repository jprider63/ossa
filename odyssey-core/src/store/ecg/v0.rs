use odyssey_crdt::CRDT;
use rand::Rng;
use std::{fmt::Debug, marker::PhantomData};

use crate::store::ecg::{ECGBody, ECGHeader};

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct HeaderId<Hash>(Hash);

// TODO: Move this to the right location.
/// An ECG header.
#[derive(Clone)]
pub struct Header<Hash, T> {
    /// A nonce to randomize the header.
    nonce: u8,

    /// The header ids of our parents in the ECG graph.
    parent_ids: Vec<HeaderId<Hash>>,

    /// The number of operations in the corresponding body.
    /// The maximum number of operations is 256.
    operations_count: u8,

    /// The hash of (batched) operations in the corresponding body.
    /// TODO: Eventually this should be of the encrypted body..
    operations_hash: Hash,

    phantom: PhantomData<T>,
}

pub struct Body<Hash, T: CRDT> {
    /// The operations in this ECG body.
    operations: Vec<T::Op>,

    phantom: PhantomData<Hash>,
}

impl<Hash: Clone + Copy + Debug + Ord, T: CRDT> ECGHeader for Header<Hash, T> {
    type HeaderId = HeaderId<Hash>;
    type Body = Body<Hash, T>;

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

    // TODO: Move this to ECG state?
    fn new_header(parents: Vec<Self::HeaderId>, body: &Body<Hash, T>) -> Self {
        let mut rng = rand::thread_rng();
        let nonce = rng.gen();

        // TODO: Check for hash conflicts and generate another nonce?

        Header {
            parent_ids: parents,
            nonce,
            operations_count: body.operations_count(),
            operations_hash: body.get_hash(),
            phantom: PhantomData,
        }
    }
}



const MAX_OPERATION_COUNT: usize = 256;
impl<Hash, T: CRDT> Body<Hash, T> {
    pub fn new(operations: Vec<T::Op>) -> Self {
        if operations.len() > MAX_OPERATION_COUNT {
            panic!("Exceeded the maximum number of batched operations.");
        }

        Body {
            operations,
            phantom: PhantomData
        }
    }
}

impl<Hash, T:CRDT> ECGBody for Body<Hash, T> {
    type Operation = T::Op;
    type Hash = Hash;

    fn operations(self) -> impl Iterator<Item = Self::Operation> {
        self.operations.into_iter()
    }
    
    fn operations_count(&self) -> u8 {
        self.operations.len().try_into().expect("Unreachable: Length is bound by MAX_OPERATION_COUNT.")
    }
    
    fn get_hash(&self) -> Self::Hash {
        todo!()
    }
}

// TODO: OperationID's are header ids and index (HeaderId, u8)
pub struct OperationId<H: ECGHeader> {
    header_id: H::HeaderId,
    operation_position: u8,
}

// JP: Should this be added to the ECGHeader trait?
pub fn zip_operations_with_time<H: ECGHeader>(header: &H, body: H::Body) -> impl Iterator<Item = (OperationId<H>, <H::Body as ECGBody>::Operation)>
where
    H::Body: ECGBody,
{
    let header_id = header.get_header_id();
    let operations_c = body.operations_count();
    (0..operations_c).zip(body.operations()).map(move |(i, op)| {
        let operation_id = OperationId {
            header_id, 
            operation_position: i,
        };
        (operation_id, op)
    })
}


#[derive(Clone, Debug)]
pub struct TestHeader {
    header_id: u32,
    parent_ids: Vec<u32>,
}

// For testing, just have the header store the parent ids.
impl ECGHeader for TestHeader {
    type HeaderId = u32;
    type Body = ();

    fn get_parent_ids(&self) -> &[u32] {
        &self.parent_ids
    }

    fn get_header_id(&self) -> u32 {
        self.header_id
    }

    fn validate_header(&self, header_id: Self::HeaderId) -> bool {
        true
    }

    fn new_header(parents: Vec<Self::HeaderId>, _body: &()) -> Self {
        todo!()
    }
}
