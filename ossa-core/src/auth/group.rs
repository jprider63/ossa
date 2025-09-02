use std::collections::BTreeMap;

use crate::{auth::identity::IdentityId, store::{bft::{Round, SCDT}, StoreRef}, util::Sha256Hash};

pub type GroupId = StoreRef<Sha256Hash, Group, ()>;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum MemberId {
    User(IdentityId),
    Group(GroupId),
    Public,
}

/// Access control role for group members.
pub enum Role {
    Relay,
    Read,
    Commenter,
    Write,
    Admin,
}

/// Member information like their permissions and the BFT round of their group or identity store.
pub struct MemberInfo {
    permissions: Role,
    round: Round,
}

/// A Group is a set of members and subgroups.
pub struct Group {
    members: BTreeMap<MemberId, MemberInfo>,
}

pub enum GroupOp {
    AddMember {
        member: MemberId,
        permissions: Role,
        round: Round,
    },
    RemoveMember {
        member: MemberId,
    },
    MemberUpdated {
        member: MemberId,
        round: Round,
    },
    UpdateMember {
        member: MemberId,
        permissions: Role,
    },
}

impl SCDT for Group {
    type Operation = GroupOp;

    fn update(mut self, op: Self::Operation) -> Self {
        match op {
            GroupOp::AddMember { member, permissions, round } => {
                let m = MemberInfo {
                    permissions,
                    round,
                };
                let _r = self.members.insert(member, m);
                debug_assert!(_r.is_none(), "Member already existed.");
            }
            GroupOp::RemoveMember { member } => {
                let _r = self.members.remove(&member);
                debug_assert!(_r.is_some());
            }
            GroupOp::MemberUpdated { member, round } => {
                if let Some(m) = self.members.get_mut(&member) {
                    m.round = round;
                } else {
                    debug_assert!(false, "Member doesn't exist");
                }
            }
            GroupOp::UpdateMember { member, permissions } => {
                if let Some(m) = self.members.get_mut(&member) {
                    m.permissions = permissions;
                } else {
                    debug_assert!(false, "Member doesn't exist");
                }
            }
        }

        self
    }

    fn is_valid_operation(self, op: Self::Operation) -> bool {
        match op {
            GroupOp::AddMember { member, .. } => {
                !self.members.contains_key(&member)
            }
            GroupOp::RemoveMember { member } => {
                self.members.contains_key(&member)
            }
            GroupOp::MemberUpdated { member, round } => {
                if let Some(m) = self.members.get(&member) {
                    m.round <= round
                } else {
                    false
                }
            }
            GroupOp::UpdateMember { member, .. } => {
                self.members.contains_key(&member)
            }
        }
    }
}

