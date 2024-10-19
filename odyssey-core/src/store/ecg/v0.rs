use odyssey_crdt::{time::CausalState, CRDT};
use rand::Rng;
use serde::{
    ser::{SerializeStruct, Serializer},
    Deserialize, Serialize,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    marker::PhantomData,
};
use typeable::Typeable;

use crate::{
    store::ecg::{self, ECGBody, ECGHeader},
    util,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct HeaderId<Hash>(Hash);

// TODO: Move this to the right location.
/// An ECG header.
#[derive(Clone, Serialize)]
// #[derive(Clone, Typeable)]
// #[tag = "v1"]
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

// TODO: Define CBOR properly
impl<Hash, T: CRDT> Serialize for Body<Hash, T>
where
    <T as CRDT>::Op: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Body", 1)?;
        s.serialize_field("operations", &self.operations)?;
        s.end()
    }
}

impl<
        Hash: Clone + Copy + Debug + Ord + util::Hash,
        T: CRDT<Time = OperationId<HeaderId<Hash>>>,
    > ECGHeader<T> for Header<Hash, T>
where
    <T as CRDT>::Op: Serialize,
    Hash: Serialize, // TODO
{
    type HeaderId = HeaderId<Hash>;
    type Body = Body<Hash, T>;

    fn get_parent_ids(&self) -> &[HeaderId<Hash>] {
        &self.parent_ids
    }

    fn get_header_id(&self) -> HeaderId<Hash> {
        HeaderId(tmp_hash(self))
    }

    fn validate_header(&self, header_id: HeaderId<Hash>) -> bool {
        // TODO: Actually check this.
        true
    }

    // TODO: Move this to ECG state?
    fn new_header(parents: BTreeSet<Self::HeaderId>, body: &Body<Hash, T>) -> Self {
        let mut rng = rand::thread_rng();
        let nonce = rng.gen();

        // Sort parent headers.
        let parents = parents.into_iter().collect();

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
    fn zip_operations_with_time(&self, body: Self::Body) -> Vec<(T::Time, T::Op)> {
        let times = self.get_operation_times(&body);
        let ops = body.operations();
        times.into_iter().zip(ops).collect()
    }

    fn get_operation_times(&self, body: &Self::Body) -> Vec<T::Time> {
        let header_id = Some(self.get_header_id());
        let operations_c = body.operations_count();
        (0..operations_c)
            .map(move |i| OperationId {
                header_id,
                operation_position: i,
            })
            .collect()
    }
}

const MAX_OPERATION_COUNT: usize = 256;

impl<Hash, T: CRDT> ECGBody<T> for Body<Hash, T> {
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
        self.operations
            .len()
            .try_into()
            .expect("Unreachable: Length is bound by MAX_OPERATION_COUNT.")
    }
}

impl<Hash: util::Hash, T: CRDT> Body<Hash, T>
where
    <T as CRDT>::Op: Serialize,
{
    fn get_hash(&self) -> Hash {
        tmp_hash(self)
    }
}

// TODO: Standardize how to hash/serialize. XXX
fn tmp_hash<T: Serialize, Hash: util::Hash>(x: &T) -> Hash {
    // JP: Better way to do this? Just serialize once?
    let mut h = Hash::new();
    let serialized = serde_cbor::ser::to_vec(&x).unwrap();
    Hash::update(&mut h, serialized);
    Hash::finalize(h)
}

// OperationID's are header ids and index (HeaderId, u8)
// TODO: Move this to odyssey-crdt::time??
#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OperationId<HeaderId> {
    pub header_id: Option<HeaderId>, // None when in the initial state?
    pub operation_position: u8,
}

impl<HeaderId> OperationId<HeaderId> {
    pub fn new(header_id: Option<HeaderId>, operation_position: u8) -> Self {
        OperationId {
            header_id,
            operation_position,
        }
    }
}

impl<Header: ECGHeader<T>, T: CRDT> CausalState for ecg::State<Header, T> {
    type Time = OperationId<Header::HeaderId>;

    fn happens_before(&self, a: &Self::Time, b: &Self::Time) -> bool {
        if a.header_id == b.header_id {
            a.operation_position < b.operation_position
        } else {
            if let Some(a_header_id) = &a.header_id {
                if let Some(b_header_id) = &b.header_id {
                    self.is_ancestor_of(a_header_id, b_header_id)
                        .expect("Invariant violated. Unknown header id.")
                } else {
                    // a.header_id.is_some() && b.header_id == None
                    false
                }
            } else {
                // a.header_id == None
                true
            }
        }
    }
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

impl<T: CRDT> ECGBody<T> for TestBody<T> {
    fn new_body(operations: Vec<T::Op>) -> Self {
        TestBody { operations }
    }

    fn operations(self) -> impl Iterator<Item = T::Op> {
        self.operations.into_iter()
    }

    fn operations_count(&self) -> u8 {
        self.operations
            .len()
            .try_into()
            .expect("Unreachable: Length is bound by MAX_OPERATION_COUNT.")
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

    fn new_header(parents: BTreeSet<Self::HeaderId>, _body: &Self::Body) -> Self {
        todo!()
    }

    fn zip_operations_with_time(&self, body: Self::Body) -> Vec<(T::Time, T::Op)>
    where
        Self::Body: ECGBody<T>,
    {
        let v: Vec<_> = todo!();
        v
    }

    fn get_operation_times(&self, body: &Self::Body) -> Vec<T::Time> {
        let v: Vec<_> = todo!();
        v
    }
}
