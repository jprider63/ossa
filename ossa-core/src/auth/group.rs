use std::{cmp::max, collections::BTreeMap};

use ossa_typeable::Typeable;
use serde::{Deserialize, Serialize};

use crate::{
    auth::identity::IdentityId, store::{
        bft::{Round, SCDT},
        StoreRef,
    }, time::ConcretizeTime, util::Sha256Hash
};

pub type GroupId = StoreRef<Sha256Hash, Group, ()>;

// #[derive(Serialize, Deserialize, Typeable, Clone, PartialEq, Eq, PartialOrd, Ord)]
// pub enum MemberId {
//     User(IdentityId),
//     Group(GroupId),
// }

/// Access control role for group members.
#[derive(Serialize, Deserialize, Typeable, Clone, Copy, Debug, PartialEq)]
pub enum Role {
    Relay,
    Read,
    Commenter,
    Write,
    Admin,
}

/// Member information like their permissions and the BFT round of their group or identity store.
#[derive(Serialize, Deserialize, Typeable, Debug, Clone)]
pub struct MemberInfo {
    pub permissions: Role,
    pub round: Round,
}

/// A Group is a set of members and subgroups.
#[derive(Clone, Serialize, Deserialize, Typeable, Debug)]
pub struct Group {
    pub members: BTreeMap<IdentityId, MemberInfo>,
    pub groups: BTreeMap<GroupId, MemberInfo>,
    pub public: Option<Role>,
}

impl Group {
    // TODO: Take as input list of members
    pub fn new(owner: IdentityId, public_permissions: Option<Role>) -> Self {
        let permissions = MemberInfo {
            permissions: Role::Admin,
            round: 0
        };
        Group {
            members: BTreeMap::from([(owner, permissions)]),
            groups: BTreeMap::new(),
            public: public_permissions,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum GroupOp {
    SetMemberAccess {
        /// Member whose permissions we're updating.
        member: IdentityId,
        /// If Role is None, remove the member.
        permissions: Option<Role>,
        /// Latest round of the member's SC identity store.
        round: Round,
    },
    MemberUpdated {
        member: IdentityId,
        round: Round,
    },
    // TODO: Update subgroup + public
}

impl<HeaderId> ConcretizeTime<HeaderId> for GroupOp {
    type Serialized = GroupOp;

    fn concretize_time(src: Self::Serialized, _current_header: HeaderId) -> Self {
        src
    }
}

impl SCDT for Group {
    type Op = GroupOp;

    fn update(mut self, op: Self::Op) -> Self {
        match op {
            GroupOp::SetMemberAccess {
                member,
                permissions,
                round,
            } => {
                if let Some(permissions) = permissions {
                    self.members.entry(member)
                        .and_modify(|m| {
                            m.round = max(round, m.round);
                            m.permissions = permissions;
                        })
                        .or_insert(MemberInfo { permissions, round });
                } else {
                    let _old_member_info = self.members.remove(&member);
                };
            }
            GroupOp::MemberUpdated { member, round } => {
                if let Some(m) = self.members.get_mut(&member) {
                    m.round = max(round, m.round);
                }
                // else {
                //     debug_assert!(false, "Member doesn't exist");
                // }
            }
        }

        self
    }

    // JP: Is this actually needed? Remove it?
    fn is_valid_operation(&self, op: Self::Op) -> bool {
        match op {
            // GroupOp::AddMember { member, .. } => !self.members.contains_key(&member),
            // GroupOp::RemoveMember { member } => self.members.contains_key(&member),
            GroupOp::SetMemberAccess { .. } => { true }
            GroupOp::MemberUpdated { member, round } => {
                if let Some(m) = self.members.get(&member) {
                    m.round <= round
                } else {
                    false
                }
            }
            // GroupOp::UpdateMember { member, .. } => self.members.contains_key(&member),
        }
    }
}
