use std::fmt::Debug;

use damysus_base::{
    enclave::{
        BlockHash, CommitmentHash, Enclave, EnclaveAutomata, EnclaveError, EnclaveTrusted,
        Justification, SignedCommitment,
    },
    view::View,
    Parent,
};
use id_set::ReplicaSet;
use shared_ids::ReplicaId;

#[derive(Debug)]
pub struct NoopEnclave {
    enclave_automata: EnclaveAutomata<NoopEnclaveTrusted>,
}

impl Enclave for NoopEnclave {
    type PlatformError = ();
    type PublicKey = [u8; 8];
    type Signature = [u8; 8];

    fn new(me: ReplicaId, n: u64, quorum: u64) -> Self {
        Self {
            enclave_automata: EnclaveAutomata::new(
                me,
                n,
                quorum,
                NoopEnclaveTrusted {
                    me: me.as_u64().to_le_bytes(),
                    peers: ReplicaSet::with_capacity(n),
                },
            ),
        }
    }

    fn get_public_key(&self) -> Result<Self::PublicKey, EnclaveError<Self::PlatformError>> {
        Ok(self.enclave_automata.get_enclave().me)
    }

    fn add_peer(
        &mut self,
        id: ReplicaId,
        key: Self::PublicKey,
    ) -> Result<(), EnclaveError<Self::PlatformError>> {
        if id.as_u64().to_le_bytes() != key {
            Err(EnclaveError::PlatformError(()))
        } else if self.enclave_automata.get_enclave_mut().add_peer(id) {
            Ok(())
        } else {
            Err(EnclaveError::DoubleKeyReceived)
        }
    }

    fn accumulate(
        &mut self,
        commitments: &[SignedCommitment<Self::Signature>],
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>> {
        self.enclave_automata.accumulate(commitments)
    }

    fn prepare(
        &mut self,
        block_view: View,
        block_hash: BlockHash,
        block_justification: Justification<Self::Signature>,
        block_parent: Parent,
        justification_block_hash: BlockHash,
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>> {
        self.enclave_automata.prepare(
            block_view,
            block_hash,
            block_justification,
            block_parent,
            justification_block_hash,
        )
    }

    fn re_sign_last_prepared(
        &mut self,
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>> {
        self.enclave_automata.re_sign_last_prepared()
    }

    fn verify_untrusted(
        &self,
        creator: ReplicaId,
        hash: &CommitmentHash,
        signature: &Self::Signature,
    ) -> bool {
        self.enclave_automata
            .get_enclave()
            .verify(creator, hash, signature)
            .unwrap()
    }
}

#[derive(Debug)]
struct NoopEnclaveTrusted {
    me: [u8; 8],
    peers: ReplicaSet,
}

impl NoopEnclaveTrusted {
    fn add_peer(&mut self, peer: ReplicaId) -> bool {
        if self.peers.contains(peer) {
            false
        } else {
            self.peers.insert(peer);
            true
        }
    }
}

impl EnclaveTrusted for NoopEnclaveTrusted {
    type PlatformError = ();
    type Signature = [u8; 8];

    fn sign(
        &self,
        _hash: &CommitmentHash,
    ) -> Result<Self::Signature, EnclaveError<Self::PlatformError>> {
        Ok(self.me)
    }

    fn verify(
        &self,
        creator: ReplicaId,
        _hash: &CommitmentHash,
        signature: &Self::Signature,
    ) -> Result<bool, EnclaveError<Self::PlatformError>> {
        Ok(self.peers.contains(creator) && creator.as_u64().to_le_bytes() == *signature)
    }
}
