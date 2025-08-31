use ossa_crdt::{time::CausalState, CRDT};
use ossa_typeable::Typeable;
use rand::Rng;
use serde::{
    de::{MapAccess, Visitor},
    ser::{SerializeStruct, Serializer},
    Deserialize, Serialize,
};
use std::{
    collections::BTreeSet,
    fmt::Debug,
    marker::PhantomData,
};

use crate::{
    store::dag::{self, ECGBody, ECGHeader},
    time::{CausalTime, ConcretizeTime},
    util,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Typeable, Deserialize, Serialize)]
pub struct HeaderId<Hash>(Hash);

// TODO: Move this to the right location.
/// An ECG header.
#[derive(Clone, Debug, Deserialize, Serialize)]
// #[derive(Clone, Typeable)]
// #[tag = "v1"]
pub struct Header<Hash> {
    // , T> {
    /// A nonce to randomize the header.
    nonce: u8,

    /// The header ids of our parents in the ECG graph.
    parent_ids: Vec<HeaderId<Hash>>,

    /// The number of operations in the corresponding body.
    /// The maximum number of operations is 255.
    operations_count: u8,

    /// The hash of (batched) operations in the corresponding body.
    /// TODO: Eventually this should be of the encrypted body..
    operations_hash: Hash,
    // // TODO: DeviceId and UserId of device signing? Maybe whole auth chain?
    // phantom: PhantomData<T>,
}

#[derive(Debug)]
pub struct Body<Hash, SerializedOp> {
    /// The operations in this ECG body.
    /// Invariant: <= 256 operations
    operations: Vec<SerializedOp>, // <CausalTime<T::Time>>>,
    phantom: PhantomData<fn(Hash)>,
}

// TODO: Define CBOR or rkyv properly
impl<Hash, SerializedOp> Serialize for Body<Hash, SerializedOp>
where
    SerializedOp: Serialize,
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

impl<'d, Hash, SerializedOp> Deserialize<'d> for Body<Hash, SerializedOp>
where
    SerializedOp: Deserialize<'d>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'d>,
    {
        struct SVisitor<Hash, T>(PhantomData<(Hash, T)>);

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Operations,
        }

        impl<'d, Hash, SerializedOp> Visitor<'d> for SVisitor<Hash, SerializedOp>
        where
            SerializedOp: Deserialize<'d>,
        {
            type Value = Body<Hash, SerializedOp>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct Body")
            }

            fn visit_map<M>(self, mut m: M) -> Result<Body<Hash, SerializedOp>, M::Error>
            where
                M: MapAccess<'d>,
            {
                let mut operations = None;
                while let Some(key) = m.next_key()? {
                    match key {
                        Field::Operations => {
                            if operations.is_some() {
                                return Err(serde::de::Error::duplicate_field("operations"));
                            }
                            operations = Some(m.next_value()?);
                        }
                    }
                }

                let operations =
                    operations.ok_or_else(|| serde::de::Error::missing_field("operations"))?;
                Ok(Body {
                    operations,
                    phantom: PhantomData,
                })
            }
        }

        deserializer.deserialize_struct("Body", &["operations"], SVisitor(PhantomData))
    }
}

impl<
        Hash: Clone + Copy + Debug + Ord + util::Hash,
        // T : CRDT<Time = OperationId<HeaderId<Hash>>>,
    > ECGHeader for Header<Hash>
where
    // <T as CRDT>::Op: Serialize,
    Hash: Serialize, // TODO
{
    type HeaderId = HeaderId<Hash>;
    // type Body = Body<Hash, T>;

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

    // // TODO: Move this to ECG state?
    // fn new_header(parents: BTreeSet<Self::HeaderId>, body: &Body<Hash, T>) -> Self {
    //     let mut rng = rand::thread_rng();
    //     let nonce = rng.gen();

    //     // Sort parent headers.
    //     let parents = parents.into_iter().collect();

    //     // TODO: Check for hash conflicts and generate another nonce?

    //     Header {
    //         parent_ids: parents,
    //         nonce,
    //         operations_count: body.operations_count(),
    //         operations_hash: body.get_hash(),
    //         phantom: PhantomData,
    //     }
    // }

    // // Replace OperationId<H> with T::Time? Or add another associated type to ECGHeader?
    // fn zip_operations_with_time<A>(&self, body: Self::Body) -> Vec<(A::Time, A::Op)> where A: CRDT {
    //     // let times = self.get_operation_times(&body);
    //     // let ops = body.operations();
    //     // times.into_iter().zip(ops).collect();
    //     todo!()

    // }

    // fn get_operation_times<A>(&self, body: &Self::Body) -> Vec<A::Time> where A: CRDT {
    //     // let header_id = Some(self.get_header_id());
    //     // let operations_c = body.operations_count();
    //     // (0..operations_c)
    //     //     .map(move |i| OperationId {
    //     //         header_id,
    //     //         operation_position: i,
    //     //     })
    //     //     .collect();
    //     todo!()
    // }
}

const MAX_OPERATION_COUNT: usize = 256;

// impl<Hash, T> ECGBody<T::Op, <T::Op as ConcretizeTime<T::Time>>::Serialized> for Body<Hash, <T::Op as ConcretizeTime<T::Time>>::Serialized>
impl<Hash, Op> ECGBody<Op, Op::Serialized> for Body<Hash, Op::Serialized>
where
    // T: CRDT<Time = OperationId<HeaderId<Hash>>>,
    Op: ConcretizeTime<HeaderId<Hash>>,
    Op::Serialized: Serialize,
    // T::Op: ConcretizeTime<OperationId<HeaderId<Hash>>>,
    // <T::Op as ConcretizeTime<T::Time>>::Serialized: Serialize,
    Hash: Clone + Copy + Debug + Ord + util::Hash + Serialize,
{
    type Header = Header<Hash>;

    fn new_body(operations: Vec<Op::Serialized>) -> Self {
        if operations.len() > MAX_OPERATION_COUNT {
            panic!("Exceeded the maximum number of batched operations.");
        }

        Body {
            operations,
            phantom: PhantomData,
        }
    }

    fn operations(self, header_id: HeaderId<Hash>) -> impl Iterator<Item = Op> {
        // let header_id = Some(header_id);
        self.operations.into_iter().map(move |op| {
            // let operation_id = OperationId {
            //     header_id,
            //     operation_position: i as u8,
            // };
            Op::concretize_time(op, header_id)
        })
        // self.operations.into_iter().map(move |op| op.concretize_time(|t| {
        //     match t {
        //         CausalTime::Time(t) => t,
        //         CausalTime::Current { operation_position } => OperationId {
        //             header_id: Some(header_id),
        //             operation_position,
        //         }
        //     }
        // }))
    }

    fn operations_count(&self) -> u8 {
        self.operations
            .len()
            .try_into()
            .expect("Unreachable: Length is bound by MAX_OPERATION_COUNT.")
    }

    fn new_header(&self, parents: BTreeSet<<Self::Header as ECGHeader>::HeaderId>) -> Self::Header {
        let mut rng = rand::thread_rng();
        let nonce = rng.gen();

        // Sort parent headers.
        let parents = parents.into_iter().collect();

        // TODO: Check for hash conflicts and generate another nonce?

        Header {
            parent_ids: parents,
            nonce,
            operations_count: <Self as ECGBody<Op, Op::Serialized>>::operations_count(self),
            operations_hash: self.get_hash(),
            // phantom: PhantomData,
        }
    }

    // fn zip_operations_with_time(
    //     self,
    //     header: &Self::Header,
    // ) -> Vec<(<T as CRDT>::Time, <T as CRDT>::Op<T::Time>)> {
    //     let times = self.get_operation_times(header);
    //     let ops = self.operations(header.get_header_id());
    //     times.into_iter().zip(ops).collect()
    // }

    // fn get_operation_times(&self, header: &Self::Header) -> Vec<<T as CRDT>::Time> {
    //     let header_id = Some(header.get_header_id());
    //     let operations_c = self.operations_count();
    //     (0..operations_c)
    //         .map(move |i| OperationId {
    //             header_id,
    //             operation_position: i,
    //         })
    //         .collect()
    // }
}

impl<Hash: util::Hash, SerializedOp> Body<Hash, SerializedOp>
where
    SerializedOp: Serialize,
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
// TODO: Move this to ossa-crdt::time??
#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Typeable)]
pub struct OperationId<HeaderId> {
    pub header_id: Option<HeaderId>, // None when in the initial state?
    pub operation_position: u8,
}

impl<HeaderId> ConcretizeTime<HeaderId> for OperationId<HeaderId> {
    type Serialized = CausalTime<OperationId<HeaderId>>;

    fn concretize_time(src: Self::Serialized, current_header: HeaderId) -> Self {
        match src {
            CausalTime::Current { operation_position } => OperationId {
                header_id: Some(current_header),
                operation_position,
            },
            CausalTime::Time(t) => t,
        }
    }
}

impl<HeaderId> OperationId<HeaderId> {
    pub fn new(header_id: Option<HeaderId>, operation_position: u8) -> Self {
        OperationId {
            header_id,
            operation_position,
        }
    }
}

impl<Header: ECGHeader, T: CRDT> CausalState for dag::State<Header, T> {
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
    pub header_id: u32,
    pub parent_ids: Vec<u32>,
    pub phantom: PhantomData<T>,
}

pub struct TestBody<SerializedOp> {
    operations: Vec<SerializedOp>,
}

/*
impl<T: CRDT<Time = u32>> ECGBody<T> for TestBody<<T::Op as ConcretizeTime<u32>>::Serialized>
where
    T::Op: ConcretizeTime<u32>,
//     T::Op<CausalTime<T::Time>>: Serialize + ConcretizeTime<CausalTime<T::Time>, T::Time, Target<T::Time> = T::Op<T::Time>>,
{
    type Header = TestHeader<T>;

    fn new_body(operations: Vec<<T::Op as ConcretizeTime<u32>>::Serialized>) -> Self {
        TestBody { operations }
    }

    fn operations(self, header_id: u32) -> impl Iterator<Item = T::Op> {
        self.operations.into_iter().map(move |op| T::Op::concretize_time(op, header_id))
        // self.operations.into_iter().map(|op| op.concretize_time(|t| {
        //     match t {
        //         CausalTime::Time(t) => t,
        //         CausalTime::Current { operation_position } => todo!(),
        //     }
        // }))
    }

    fn operations_count(&self) -> u8 {
        self.operations
            .len()
            .try_into()
            .expect("Unreachable: Length is bound by MAX_OPERATION_COUNT.")
    }

    // fn zip_operations_with_time(
    //     self,
    //     header: &Self::Header,
    // ) -> Vec<(<T as CRDT>::Time, <T as CRDT>::Op<T::Time>)> {
    //     todo!()
    // }

    // fn get_operation_times(&self, header: &Self::Header) -> Vec<<T as CRDT>::Time> {
    //     todo!()
    // }

    fn new_header(&self, parents: BTreeSet<<Self::Header as ECGHeader>::HeaderId>) -> Self::Header {
        todo!()
    }
}
*/

// For testing, just have the header store the parent ids.
impl<A: CRDT> ECGHeader for TestHeader<A> {
    type HeaderId = u32;
    // type Body = TestBody<A>;

    fn get_parent_ids(&self) -> &[u32] {
        &self.parent_ids
    }

    fn get_header_id(&self) -> u32 {
        self.header_id
    }

    fn validate_header(&self, header_id: Self::HeaderId) -> bool {
        true
    }

    // fn new_header(parents: BTreeSet<Self::HeaderId>, _body: &Self::Body) -> Self {
    //     todo!()
    // }

    // // JP: TODO: Move this to ECGBody?
    // fn zip_operations_with_time<T>(&self, body: Self::Body) -> Vec<(T::Time, T::Op)>
    // where
    //     T: CRDT,
    //     Self::Body: ECGBody<T>,
    // {
    //     let v: Vec<_> = todo!();
    //     v
    // }

    // fn get_operation_times<T>(&self, body: &Self::Body) -> Vec<T::Time> where T: CRDT, {
    //     let v: Vec<_> = todo!();
    //     v
    // }
}
