pub use dioxus;
use dioxus::hooks::use_context;
use dioxus::prelude::{ScopeId, Task, current_scope_id, use_hook};
use dioxus::signals::{Readable as _, Signal, Writable as _};
pub use dioxus_desktop;
use tracing::debug;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::panic::Location;
use std::rc::Rc;

use ossa_core::Ossa;
use ossa_core::core::OssaType;
use ossa_core::core::StoreHandle;
use ossa_core::store::ecg::v0::{Body, Header, HeaderId, OperationId};
use ossa_core::store::ecg::{ECGBody, ECGHeader};
use ossa_core::store::{StateUpdate, ecg};
use ossa_core::time::{CausalTime, ConcretizeTime};
use ossa_core::util::Sha256Hash;
use ossa_crdt::CRDT;

// #[derive(Props, PartialEq)]
pub struct OssaProp<A: OssaType + 'static> {
    ossa: Rc<Ossa<A>>,
}

impl<A: OssaType + 'static> Clone for OssaProp<A> {
    fn clone(&self) -> Self {
        OssaProp {
            ossa: self.ossa.clone(),
        }
    }
}

impl<A: OssaType> OssaProp<A> {
    pub fn new(ossa: Ossa<A>) -> Self {
        OssaProp {
            ossa: Rc::new(ossa),
        }
    }

    pub fn ossa(&self) -> &Rc<Ossa<A>> {
        &self.ossa
    }
}

/// A default setup that implements `OssaType` with typical settings like using sha256 as the hash function.
pub enum DefaultSetup {}

impl OssaType for DefaultSetup {
    type Hash = Sha256Hash;
    type StoreId = Sha256Hash;
    type ECGHeader = Header<Sha256Hash>;
    type ECGBody<T: CRDT<Op: ConcretizeTime<HeaderId<Sha256Hash>>>> =
        Body<Sha256Hash, <T::Op as ConcretizeTime<HeaderId<Sha256Hash>>>::Serialized>;

    type Time = OperationId<HeaderId<Sha256Hash>>;

    type CausalState<T: CRDT<Time = Self::Time>> = ecg::State<Self::ECGHeader, T>;

    fn to_causal_state<T: CRDT<Time = Self::Time>>(
        st: &ecg::State<Self::ECGHeader, T>,
    ) -> &Self::CausalState<T> {
        st
    }
}

pub struct UseStore<
    OT: OssaType + 'static,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>> + 'static,
> {
    future: Task,
    handle: Rc<RefCell<StoreHandle<OT, T>>>,
    // handle: StoreHandle<OT, T>,
    state: Signal<Option<StoreState<OT, T>>>,
    // peers, connections, etc
}

// TODO: Provide way to gracefully drop UseStore
// impl<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: Serialize> + 'static> Drop for UseStore<OT, T> {
//     fn drop(&mut self) {
//         // JP: Gracefully shutdown store receiver somehow?
//         self.future.cancel();
//
//         self.state.manually_drop(); // JP: Is this needed?
//     }
// }

impl<
    OT: OssaType + 'static,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
> Clone for UseStore<OT, T>
{
    fn clone(&self) -> Self {
        UseStore {
            future: self.future,
            handle: self.handle.clone(),
            state: self.state,
        }
    }
}

// #[derive(Clone)]
pub struct StoreState<OT: OssaType, T: CRDT<Time = OT::Time>> {
    state: T,
    ecg: ecg::State<OT::ECGHeader, T>,
}

impl<OT: OssaType, T: CRDT<Time = OT::Time>> StoreState<OT, T> {
    pub fn state(&self) -> &T {
        &self.state
    }

    pub fn ecg(&self) -> &ecg::State<OT::ECGHeader, T> {
        &self.ecg
    }
}

impl<OT: OssaType, T: CRDT<Time = OT::Time> + Clone> Clone for StoreState<OT, T>
where
    <OT as OssaType>::ECGHeader: Clone,
{
    fn clone(&self) -> Self {
        StoreState {
            state: self.state.clone(),
            ecg: self.ecg.clone(),
        }
    }
}

/*
pub fn use_store<
    OT: OssaType + 'static,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
    F,
>(
    build_store_handle: F,
) -> UseStore<OT, T>
where
    F: FnOnce(&Ossa<OT>) -> StoreHandle<OT, T>,
{
    let scope = current_scope_id().expect("Failed to get scope id");
    let ossa = use_context::<OssaProp<OT>>().ossa;
    let caller = std::panic::Location::caller();
    use_hook(|| new_store_helper(&ossa, scope, caller, build_store_handle).unwrap())
}

fn new_store_helper<
    OT: OssaType + 'static,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
    F,
>(
    ossa: &Ossa<OT>,
    scope: ScopeId,
    caller: &'static Location,
    build_store_handle: F,
) -> Option<UseStore<OT, T>>
where
    F: FnOnce(&Ossa<OT>) -> StoreHandle<OT, T>,
{
    let mut handle = build_store_handle(ossa);
    let mut recv_st = handle.subscribe_to_state();

    let mut state = Signal::new_maybe_sync_in_scope_with_caller(None, scope, caller);

    let future = spawn_in_scope(scope, async move {
        debug!("Creating future for store");
        while let Some(msg) = recv_st.recv().await {
            match msg {
                StateUpdate::Snapshot {
                    snapshot,
                    ecg_state,
                } => {
                    debug!("Received state!");
                    let s = StoreState {
                        state: snapshot,
                        ecg: ecg_state,
                    };
                    state.set(Some(s));
                }
                StateUpdate::Downloading { percent } => {
                    debug!("Store is downloading ({percent}%)");
                    state.set(None);
                }
            }
        }
        debug!("Future for store is exiting");
    });

    let Some(future) = future else {
        state.manually_drop(); // JP: Is this needed?
        return None;
    };

    let handle = Rc::new(RefCell::new(handle));
    Some(UseStore {
        future,
        handle,
        state,
    })
}

pub fn new_store_in_scope<
    OT: OssaType + 'static,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
    F,
>(
    scope: ScopeId,
    build_store_handle: F,
) -> Option<UseStore<OT, T>>
where
    F: FnOnce(&Ossa<OT>) -> StoreHandle<OT, T>,
{
    let ossa = use_context::<OssaProp<OT>>().ossa;
    let caller = std::panic::Location::caller();
    new_store_helper(&ossa, scope, caller, build_store_handle)
}
*/

pub struct OperationBuilder<
    OT: OssaType,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
> {
    handle: Rc<RefCell<StoreHandle<OT, T>>>,
    parents: BTreeSet<<<OT as OssaType>::ECGHeader as ECGHeader>::HeaderId>,
    operations: Vec<<T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized>,
}

const MAX_OPS: usize = 256; // TODO: This is already defined somewhere else?
impl<
    OT: OssaType,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
> OperationBuilder<OT, T>
{
    /// Queue up an operation to apply.
    /// Cannot queue up more than 256 operations inside a single queue.
    /// This function will return `None` if more than 256 operations are queued.
    /// Only refer to the returned time in other operations inside this queue, otherwise the time reference will be incorrect.
    pub fn queue<F>(&mut self, f: F) -> Option<CausalTime<OT::Time>>
    where
        OT::Time: Clone,
        F: FnOnce(
            CausalTime<OT::Time>,
        )
            -> <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
    {
        if self.operations.len() >= MAX_OPS {
            return None;
        }
        let t = CausalTime::current_time((self.operations.len()) as u8);
        self.operations.push(f(t.clone()));

        Some(t)
    }

    pub fn apply(self) -> <OT::ECGHeader as ECGHeader>::HeaderId
    where
        // T::Op<CausalTime<OT::Time>>: Serialize,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: ECGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >,
    {
        (*self.handle)
            .borrow_mut()
            .apply_batch(self.parents, self.operations)
    }
}

impl<
    OT: OssaType,
    T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>,
> UseStore<OT, T>
{
    pub fn get_current_state(&self) -> Option<T>
    where
        T: Clone,
        <OT as OssaType>::ECGHeader: Clone,
    {
        self.state.cloned().map(|s| s.state)
    }

    pub fn get_current_store_state(&self) -> Option<StoreState<OT, T>>
    where
        T: Clone,
        <OT as OssaType>::ECGHeader: Clone,
    {
        self.state.cloned()
    }

    /// Applies an operation to the Store's CRDT with the closure the builds the operation. If  you want to apply multiple operations, use `operation_builder`.
    pub fn apply<F>(&self, op: F) -> OperationId<<OT::ECGHeader as ECGHeader>::HeaderId>
    where
        F: FnOnce(
            CausalTime<OT::Time>,
        )
            -> <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: ECGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >,
    {
        let parent_header_ids = {
            let cookbook_store_state = self.state.peek();
            let cookbook_store_state = cookbook_store_state.as_ref().expect("TODO");
            cookbook_store_state.ecg.tips().clone()
        };
        self.apply_with_parents(parent_header_ids, op)
    }

    pub fn apply_with_parents<F>(
        &self,
        parents: BTreeSet<<<OT as OssaType>::ECGHeader as ECGHeader>::HeaderId>,
        op: F,
    ) -> OperationId<<OT::ECGHeader as ECGHeader>::HeaderId>
    where
        F: FnOnce(
            CausalTime<OT::Time>,
        )
            -> <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: ECGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >,
    {
        let t = CausalTime::current_time(0);
        let header_id = (*self.handle).borrow_mut().apply(parents, op(t));

        OperationId {
            header_id: Some(header_id),
            operation_position: 0,
        }
    }

    // pub fn apply_batch(
    //     &mut self,
    //     parents: BTreeSet<<<OT as OssaType>::ECGHeader as ECGHeader>::HeaderId>,
    //     op: Vec<T::Op<CausalTime<OT::Time>>>,
    // ) -> <OT::ECGHeader as ECGHeader>::HeaderId
    // where
    //     OT::ECGBody<T>: ECGBody<T, Header = OT::ECGHeader>,
    // {
    //     (*self.handle).borrow_mut().apply_batch(parents, op)
    // }

    pub fn operations_builder(&self) -> OperationBuilder<OT, T>
    where
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
    {
        let cookbook_store_state = self.state.peek();
        let cookbook_store_state = cookbook_store_state.as_ref().expect("TODO");
        let parent_header_ids = cookbook_store_state.ecg.tips().clone();

        OperationBuilder {
            handle: self.handle.clone(),
            parents: parent_header_ids,
            operations: vec![],
        }
    }
}
