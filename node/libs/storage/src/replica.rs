//! This is module is used to store the replica state. The main purpose of this is to act as a backup in case the node crashes.

use crate::{
    types::{DatabaseKey, ReplicaState},
    Storage,
};

impl Storage {
    // ---------------- Read methods ----------------

    /// Gets the replica state, if it is contained in the database.
    pub fn get_replica_state(&self) -> Option<ReplicaState> {
        self.read()
            .get(DatabaseKey::ReplicaState.encode_key())
            .unwrap()
            .map(|b| schema::decode(&b).expect("Failed to decode replica state!"))
    }

    // ---------------- Write methods ----------------

    /// Store the given replica state into the database.
    pub fn put_replica_state(&self, replica_state: &ReplicaState) {
        self.write()
            .put(
                DatabaseKey::ReplicaState.encode_key(),
                schema::encode(replica_state),
            )
            .unwrap();
    }
}
