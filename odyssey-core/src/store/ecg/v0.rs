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

    // TODO: DeviceId and UserId of device signing? Maybe whole auth chain?

    phantom: PhantomData<T>,
}

pub struct Body<Hash, T: CRDT> {
    /// The operations in this ECG body.
    operations: Vec<T::Op>,
    phantom: PhantomData<Hash>,
}

impl<Hash: Clone + Copy + Debug + Ord, T: CRDT<Time = OperationId<HeaderId<Hash>>>> ECGHeader<T> for Header<Hash, T> {
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

    // Replace OperationId<H> with T::Time? Or add another associated type to ECGHeader?
    fn zip_operations_with_time(&self, body: Self::Body) -> impl Iterator<Item = (T::Time, T::Op)>
    {
        let header_id = self.get_header_id();
        let operations_c = body.operations_count();
        (0..operations_c).zip(body.operations()).map(move |(i, op)| {
            let operation_id = OperationId {
                header_id,
                operation_position: i,
            };
            (operation_id, op)
        })
    }

}



const MAX_OPERATION_COUNT: usize = 256;

impl<Hash, T:CRDT> ECGBody<T> for Body<Hash, T> {
    type Hash = Hash;

    fn new_body(operations: Vec<T::Op>) -> Self {
        if operations.len() > MAX_OPERATION_COUNT {
            panic!("Exceeded the maximum number of batched operations.");
        }

        Body {
            operations,
            phantom: PhantomData,
        }
    }

    fn operations(self) -> impl Iterator<Item = T::Op> {
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
pub struct OperationId<HeaderId> {
    header_id: HeaderId,
    operation_position: u8,
}


#[derive(Clone, Debug)]
pub struct TestHeader<T> {
    header_id: u32,
    parent_ids: Vec<u32>,
    phantom: PhantomData<T>,
}

pub struct TestBody<T: CRDT> {
    operations: Vec<T::Op>,
}

impl<T:CRDT> ECGBody<T> for TestBody<T> {
    type Hash = (); // JP: Maybe ECGBody should not have Hash since it's not used by all impls???

    fn new_body(operations: Vec<T::Op>) -> Self {
        TestBody {
            operations,
        }
    }

    fn operations(self) -> impl Iterator<Item = T::Op> {
        self.operations.into_iter()
    }

    fn operations_count(&self) -> u8 {
        self.operations.len().try_into().expect("Unreachable: Length is bound by MAX_OPERATION_COUNT.")
    }

    fn get_hash(&self) -> Self::Hash {
        ()
    }
}

// For testing, just have the header store the parent ids.
impl<T: CRDT> ECGHeader<T> for TestHeader<T> {
    type HeaderId = u32;
    type Body = TestBody<T>;

    fn get_parent_ids(&self) -> &[u32] {
        &self.parent_ids
    }

    fn get_header_id(&self) -> u32 {
        self.header_id
    }

    fn validate_header(&self, header_id: Self::HeaderId) -> bool {
        true
    }

    fn new_header(parents: Vec<Self::HeaderId>, _body: &Self::Body) -> Self {
        todo!()
    }

    fn zip_operations_with_time(&self, body: Self::Body) -> impl Iterator<Item = (T::Time, T::Op)>
    where
        Self::Body: ECGBody<T>,
    {
        let v: Vec<_> = todo!();
        v.into_iter()
    }
}
