
pub use dioxus;
use dioxus::hooks::use_context;
use dioxus::prelude::{current_scope_id, spawn_in_scope, use_hook, ScopeId, Task};
use dioxus::signals::{Readable as _, Signal, Writable as _};
pub use dioxus_desktop;
use tracing::debug;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::panic::Location;
use std::rc::Rc;

use ossa_core::core::{StoreHandle};
use ossa_core::store::ecg::v0::{Body, Header, HeaderId, OperationId};
use ossa_core::store::ecg::{ECGBody, ECGHeader};
use ossa_core::store::{ecg, StateUpdate};
use ossa_core::core::OssaType;
use ossa_core::time::{CausalTime, ConcretizeTime};
use ossa_core::util::Sha256Hash;
use ossa_core::Ossa;
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
}

pub enum DefaultSetup {}

impl OssaType for DefaultSetup {
    type Hash = Sha256Hash;
    type StoreId = Sha256Hash; // TODO
                       // type ECGHeader<T: CRDT<Op: Serialize, Time = OperationId<HeaderId<Sha256Hash>>>> = Header<Sha256Hash, T>;
                       // type ECGHeader<T: CRDT<Time = OperationId<HeaderId<Sha256Hash>>>> = Header<Sha256Hash, T>;
    type ECGHeader = Header<Sha256Hash>;
    // type ECGHeader<T: CRDT<Op: Serialize>> = Header<Sha256Hash, T>;
    type ECGBody<T: CRDT<Op: ConcretizeTime<HeaderId<Sha256Hash>>>> = Body<Sha256Hash, <T::Op as ConcretizeTime<HeaderId<Sha256Hash>>>::Serialized>; // : CRDT<Time = Self::Time, Op: Serialize>> = Body<Sha256Hash, T>;

    type Time = OperationId<HeaderId<Sha256Hash>>;

    type CausalState<T: CRDT<Time = Self::Time>> = ecg::State<Self::ECGHeader, T>;
    // type ECGHeader<T> = Header<Sha256Hash, T>
    //     where T: CRDT<Time = OperationId<HeaderId<Sha256Hash>>>;
    // type ECGHeader<T> = Header<Sha256Hash, T>;

    fn to_causal_state<T: CRDT<Time = Self::Time>>(
        st: &ecg::State<Self::ECGHeader, T>,
    ) -> &Self::CausalState<T> {
        st
    }
}

// TODO: Create `odyssey-dioxus` crate?
pub struct UseStore<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>> + 'static> {
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

impl<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>> Clone for UseStore<OT, T> {
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

pub fn use_store<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>, F>(
    build_store_handle: F,
) -> UseStore<OT, T>
where
    F: FnOnce(&Ossa<OT>) -> StoreHandle<OT, T>,
{
    let scope = current_scope_id().expect("Failed to get scope id");
    let odyssey = use_context::<OssaProp<OT>>().ossa;
    let caller = std::panic::Location::caller();
    use_hook(|| new_store_helper(&odyssey, scope, caller, build_store_handle).unwrap())
}

fn new_store_helper<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>, F>(
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

    // use_hook(|| Signal::new_maybe_sync_in_scope_with_caller(f(), scope, caller))
    let mut state = Signal::new_maybe_sync_in_scope_with_caller(None, scope, caller);

    let future = spawn_in_scope(scope, async move {
        debug!("Creating future for store");
        //     let mut recv_state = handle2.subscribe_to_state();
        //     // let mut recv_state = Rc::try_unwrap(recv_state).unwrap();
        //     // let mut recv_state = recv_state.clone();
        //     // let recv_state = Rc::get_mut(&mut recv_state).unwrap();
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
                StateUpdate::Downloading {percent} => {
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
    Some(UseStore { future, handle, state })
}

pub fn new_store_in_scope<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>, F>(
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

/*
fn use_signal_in_scope<T: 'static>(scope: ScopeId, f: impl FnOnce() -> T) -> Signal<T, UnsyncStorage> {
    let caller = std::panic::Location::caller();
    use_hook(|| Signal::new_maybe_sync_in_scope_with_caller(f(), scope, caller))
}

// JP: Is this usage of ScopeId correct? Will this be deallocated properly?
fn use_store_in_scope<OT: OssaType + 'static, T: CRDT<Time = OT::Time, Op: Serialize>, F>(
    scope: ScopeId,
    build_store_handle: F,
) -> UseStore<OT, T>
where
    F: FnOnce(&Odyssey<CookbookApplication>) -> StoreHandle<OT, T>,
{
    // JP: How do we put this inside the `use_hook`?
    let odyssey = use_context::<OssaProp<CookbookApplication>>().odyssey;
    let (handle, mut recv_state) = use_hook(|| {
        let mut h = build_store_handle(&odyssey);
        let recv_st = h.subscribe_to_state();

        // // Get current state.
        // let st = recv_st.blocking_recv().unwrap();

        let h = Rc::new(RefCell::new(h)); // JP: Annoyingly required since dioxus requires clone... XXX
                                          // let st = Rc::new(st); // JP: Annoyingly required since dioxus requires clone... XXX
                                          // let recv_st = Rc::new(recv_st); // JP: Annoyingly required since dioxus requires clone... XXX
        let recv_st = CopyValue::new_in_scope(recv_st, scope); // JP: Annoyingly required since dioxus requires clone... XXX
        (h, recv_st)
    });
    // let mut state = use_signal(|| None);
    let mut state = use_signal_in_scope(scope, || None);
    // let mut state: Signal<T> = use_signal(|| {
    //     // let recv_state = Rc::get_mut(&mut recv_state).unwrap();
    //     let init_st = recv_state.write().blocking_recv().unwrap();
    //     init_st
    //     // Rc::into_inner(init_st).unwrap()
    // });
    // let handle2 = handle.clone();
    // TODO...
    let future = use_future(move || async move {
        debug!("Creating future for store");
        //     let mut recv_state = handle2.subscribe_to_state();
        //     // let mut recv_state = Rc::try_unwrap(recv_state).unwrap();
        //     // let mut recv_state = recv_state.clone();
        //     // let recv_state = Rc::get_mut(&mut recv_state).unwrap();
        while let Some(msg) = recv_state.write().recv().await {
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
                StateUpdate::Downloading {percent} => {
                    debug!("Store is downloading ({percent}%)");
                    state.set(None);
                }
            }
        }
        debug!("Future for store is exiting");
    });
    UseStore { future, handle, state }
}
*/

pub struct OperationBuilder<OT: OssaType, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>> {
    handle: Rc<RefCell<StoreHandle<OT, T>>>,
    parents: BTreeSet<<<OT as OssaType>::ECGHeader as ECGHeader>::HeaderId>,
    operations: Vec<<T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized>,
}

const MAX_OPS: usize = 256; // TODO: This is already defined somewhere else?
impl<OT: OssaType, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>> OperationBuilder<OT, T> {
    /// Queue up an operation to apply.
    /// Cannot queue up more than 256 operations inside a single queue.
    /// This function will return `None` if more than 256 operations are queued.
    /// Only refer to the returned time in other operations inside this queue, otherwise the time reference will be incorrect. 
    pub fn queue<F>(&mut self, f: F) -> Option<CausalTime<OT::Time>>
    where
        OT::Time: Clone,
        F: FnOnce(CausalTime<OT::Time>) -> <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
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
        OT::ECGBody<T>: ECGBody<T::Op, <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized, Header = OT::ECGHeader>,
    {
        (*self.handle).borrow_mut().apply_batch(self.parents, self.operations)

        // let mut ops = vec![];
        // let mut ret = vec![];
        // for op in self.operations {
        //     if ops.len() >= MAX_OPS {
        //         ret.push((*self.handle).borrow_mut().apply_batch(self.parents.clone(), ops));
        //         ops = vec![];
        //     }

        //     ops.push(op);
        // }

        // if !ops.is_empty() {
        //     ret.push((*self.handle).borrow_mut().apply_batch(self.parents, ops));
        // }

        // ret
    }
}

impl<OT: OssaType, T: CRDT<Time = OT::Time, Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>> UseStore<OT, T>
// where
//     T::Op<CausalTime<OT::Time>>: Serialize,
{
    // TODO: Apply operations, get current state, etc
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

    // fn apply_helper(
    //     &self,
    //     op: T::Op<CausalTime<OT::Time>>,
    // ) -> <OT::ECGHeader as ECGHeader>::HeaderId
    // where
    //     OT::ECGBody<T>: ECGBody<T, Header = OT::ECGHeader>,
    // {
    //     let parent_header_ids = {
    //         let cookbook_store_state = self.state.peek();
    //         let cookbook_store_state = cookbook_store_state.as_ref().expect("TODO");
    //         cookbook_store_state.ecg.tips().clone()
    //     };
    //     (*self.handle).borrow_mut().apply(parent_header_ids, op)
    // }

    /// Applies an operation to the Store's CRDT with the closure the builds the operation. If  you want to apply multiple operations, use `operation_builder`.
    pub fn apply<F>(
        &self,
        op: F,
    ) -> OperationId<<OT::ECGHeader as ECGHeader>::HeaderId>
    where
        F: FnOnce(CausalTime<OT::Time>) -> <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: ECGBody<T::Op, <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized, Header = OT::ECGHeader>,
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
        F: FnOnce(CausalTime<OT::Time>) -> <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: ECGBody<T::Op, <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized, Header = OT::ECGHeader>,
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
