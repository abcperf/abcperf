use std::fmt::{Debug, Display};

use blake2::{Blake2b512, Digest};
use hashbar::{Hashbar, Hasher};
use heaps::Keyable;
use id_set::ReplicaSet;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use tracing::debug;

use crate::view::View;
use crate::Parent;

#[derive(Debug)]
pub enum EnclaveError<E: Debug> {
    JustificationIsGenesis,
    JustificationInvalidView {
        justification_view: View,
        view_usable_for: View,
        enclave_view: View,
        justification_block_view: View,
    },
    BlockToPrepareIsForWrongView {
        enclave_view: View,
        block_view: View,
    },
    JustificationBlockHashDoesNotMatchCommitmentBlockHash {
        commitment_block_hash: BlockHash,
        justification_block_hash: BlockHash,
    },
    InvalidAccumulatorSignature,
    IncompatibleCommitmentForAccumulation,
    NonDescendingOrderOfCommitmentsForAccumulation,
    NotEnoughValidCommitmentsForAccumulation,
    DoubleKeyReceived,
    PlatformError(E),
}

#[derive(PartialEq, Eq, Serialize, Deserialize, Clone, PartialOrd, Ord, Debug)]
pub struct BlockHash(#[serde(with = "serde_bytes")] [u8; 32]);

impl BlockHash {
    pub fn genesis() -> Self {
        Self([0; 32])
    }
}

impl AsRef<[u8]> for BlockHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Hashbar for BlockHash {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0);
    }
}

impl<T: Into<[u8; 32]>> From<T> for BlockHash {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl Display for BlockHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut self_hash = String::from("");
        for e in self.0.iter() {
            self_hash.push_str(&format!("{e:#04x}")[2..]);
        }
        write!(f, "{self_hash}")
    }
}

#[derive(PartialEq, Eq, Serialize, Deserialize, Clone, PartialOrd, Ord, Debug)]
pub struct CommitmentHash(#[serde(with = "serde_bytes")] [u8; 64]);

impl AsRef<[u8]> for CommitmentHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Hashbar for CommitmentHash {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0);
    }
}

impl<T: Into<[u8; 64]>> From<T> for CommitmentHash {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl Display for CommitmentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut self_hash = String::from("");
        for e in self.0.iter() {
            self_hash.push_str(&format!("{e:#04x}")[2..]);
        }
        write!(f, "{self_hash}")
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Justification<Sig> {
    Genesis,
    Quorum(SignedCommitment<Sig>),
}

impl<Sig> Justification<Sig> {
    pub fn get_commitment(&self) -> Option<&SignedCommitment<Sig>> {
        if let Self::Quorum(signed_commitment) = self {
            Some(signed_commitment)
        } else {
            None
        }
    }

    pub fn is_genesis(&self) -> bool {
        matches!(self, Self::Genesis)
    }
}

impl<Sig: AsRef<[u8]>> Hashbar for Justification<Sig> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            Self::Genesis => hasher.update(&[0]),
            Self::Quorum(signed_commitment) => {
                hasher.update(&[1]);
                signed_commitment.commitment.hash_value().hash(hasher);
                hasher.update(signed_commitment.signature.as_ref());
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SignedCommitment<Sig> {
    commitment: Commitment,
    signature: Sig,
}

#[allow(dead_code)]
impl<Sig> SignedCommitment<Sig> {
    fn new(commitment: Commitment, signature: Sig) -> Self {
        Self {
            commitment,
            signature,
        }
    }

    pub fn get_creator(&self) -> ReplicaId {
        self.commitment.creator
    }

    pub fn get_view(&self) -> View {
        self.commitment.view
    }

    pub fn get_view_usable_for(&self) -> View {
        self.get_view().get_successor()
    }

    pub fn get_block_view(&self) -> View {
        if self.commitment.commitment_type == CommitmentType::Accumulator
            || self.commitment.commitment_phase == CommitmentPhase::Prepare
        {
            self.commitment.block_view
        } else {
            self.commitment.prepared_block_view
        }
    }

    pub fn get_block_hash(&self) -> &BlockHash {
        if self.commitment.commitment_type == CommitmentType::Accumulator
            || self.commitment.commitment_phase == CommitmentPhase::Prepare
        {
            &self.commitment.block_hash
        } else {
            &self.commitment.prepared_block_hash
        }
    }

    pub fn get_prepared_block_view(&self) -> View {
        self.commitment.prepared_block_view
    }

    pub(crate) fn get_prepared_block_hash(&self) -> &BlockHash {
        &self.commitment.prepared_block_hash
    }

    pub fn is_prepare(&self) -> bool {
        self.commitment.commitment_phase == CommitmentPhase::Prepare
    }

    pub fn is_usig(&self) -> bool {
        self.commitment.is_usig()
    }

    pub fn is_accumulator(&self) -> bool {
        self.commitment.is_accumulator()
    }

    pub fn hash_value(&self) -> &CommitmentHash {
        self.commitment.hash_value()
    }

    pub fn get_signature(&self) -> &Sig {
        &self.signature
    }
}

impl<Sig> Keyable for SignedCommitment<Sig> {
    type Key = (View, ReplicaId);
    fn key(&self) -> Self::Key {
        (self.commitment.view, self.commitment.creator)
    }
}

impl<Sig> Ord for SignedCommitment<Sig> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.commitment.view.eq(&other.commitment.view) {
            self.commitment.creator.cmp(&other.commitment.creator)
        } else {
            self.commitment.view.cmp(&other.commitment.view)
        }
    }
}

impl<Sig> Eq for SignedCommitment<Sig> {}

impl<Sig> PartialEq for SignedCommitment<Sig> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl<Sig> PartialOrd for SignedCommitment<Sig> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Commitment {
    creator: ReplicaId,
    view: View,
    prepared_block_view: View,
    prepared_block_hash: BlockHash,
    block_view: View,
    block_hash: BlockHash,
    commitment_phase: CommitmentPhase,
    commitment_type: CommitmentType,
    #[serde(skip)]
    precomputed_hash: OnceCell<CommitmentHash>,
}

impl Commitment {
    fn hash_value(&self) -> &CommitmentHash {
        self.precomputed_hash.get_or_init(|| {
            let mut hasher = Blake2b512::new();
            self.creator.hash(&mut hasher);
            self.view.hash(&mut hasher);
            self.prepared_block_view.hash(&mut hasher);
            self.commitment_phase.hash(&mut hasher);
            self.commitment_type.hash(&mut hasher);
            hasher.finalize().into()
        })
    }

    fn is_usig(&self) -> bool {
        self.commitment_type == CommitmentType::Usig
    }

    fn is_accumulator(&self) -> bool {
        self.commitment_type == CommitmentType::Accumulator
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum CommitmentPhase {
    NewView,
    Prepare,
}

impl Hashbar for CommitmentPhase {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            Self::NewView => hasher.update(&[0]),
            Self::Prepare => hasher.update(&[1]),
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum CommitmentType {
    Accumulator,
    Usig,
}

impl Hashbar for CommitmentType {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            Self::Accumulator => hasher.update(&[0]),
            Self::Usig => hasher.update(&[1]),
        }
    }
}

pub trait Enclave {
    type PublicKey: Clone + Debug + for<'a> Deserialize<'a> + Serialize;
    type Signature: Clone + Debug + for<'a> Deserialize<'a> + Serialize + AsRef<[u8]>;
    type PlatformError: Send + Debug;

    fn new(me: ReplicaId, n: u64, quorum: u64) -> Self;
    fn get_public_key(&self) -> Result<Self::PublicKey, EnclaveError<Self::PlatformError>>;
    fn add_peer(
        &mut self,
        id: ReplicaId,
        key: Self::PublicKey,
    ) -> Result<(), EnclaveError<Self::PlatformError>>;

    fn accumulate(
        &mut self,
        commitments: &[SignedCommitment<Self::Signature>],
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>>;

    fn prepare(
        &mut self,
        block_view: View,
        block_hash: BlockHash,
        block_justification: Justification<Self::Signature>,
        block_parent: Parent,
        justification_block_hash: BlockHash,
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>>;

    fn re_sign_last_prepared(
        &mut self,
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>>;

    fn verify_untrusted(
        &self,
        creator: ReplicaId,
        hash: &CommitmentHash,
        signature: &Self::Signature,
    ) -> bool;
}

pub trait EnclaveTrusted {
    type Signature: Clone + Debug + for<'a> Deserialize<'a> + Serialize + AsRef<[u8]>;
    type PlatformError: Send + Debug;

    fn sign(
        &self,
        hash: &CommitmentHash,
    ) -> Result<Self::Signature, EnclaveError<Self::PlatformError>>;
    fn verify(
        &self,
        creator: ReplicaId,
        hash: &CommitmentHash,
        signature: &Self::Signature,
    ) -> Result<bool, EnclaveError<Self::PlatformError>>;
}

#[derive(Debug)]
pub struct EnclaveAutomata<Enc: EnclaveTrusted> {
    enclave: Enc,
    me: ReplicaId,
    n: u64,
    quorum: u64,
    view: View,
    last_prepared_block_hash: BlockHash,
    last_prepared_block_view: View,
}

impl<Enc: EnclaveTrusted> EnclaveAutomata<Enc> {
    pub fn new(me: ReplicaId, n: u64, quorum: u64, enclave: Enc) -> Self {
        Self {
            enclave,
            me,
            n,
            quorum,
            view: View::default(),
            last_prepared_block_hash: BlockHash::genesis(),
            last_prepared_block_view: View::default(),
        }
    }

    pub fn get_enclave_mut(&mut self) -> &mut Enc {
        &mut self.enclave
    }

    pub fn get_enclave(&self) -> &Enc {
        &self.enclave
    }

    pub fn accumulate(
        &mut self,
        commitments: &[SignedCommitment<Enc::Signature>],
    ) -> Result<SignedCommitment<Enc::Signature>, EnclaveError<Enc::PlatformError>> {
        let view = self.view.get_predecessor().unwrap();
        let mut is_prepare = true;
        let mut valid = 0u64;
        let mut seen = ReplicaSet::with_capacity(self.n);

        let (prepared_block_view, prepared_block_hash, mut block_view, mut block_hash) =
            match commitments.last() {
                Some(c) => (
                    c.get_prepared_block_view(),
                    c.get_prepared_block_hash(),
                    c.get_block_view(),
                    c.get_block_hash(),
                ),
                None => return Err(EnclaveError::NotEnoughValidCommitmentsForAccumulation),
            };

        for signed_commitment in commitments.iter().rev() {
            if seen.contains(signed_commitment.commitment.creator) {
                return Err(EnclaveError::IncompatibleCommitmentForAccumulation);
            }

            if signed_commitment.get_prepared_block_view() > prepared_block_view {
                return Err(EnclaveError::NonDescendingOrderOfCommitmentsForAccumulation);
            }

            if !matches!(
                signed_commitment.commitment.commitment_phase,
                CommitmentPhase::Prepare { .. },
            ) {
                block_view = prepared_block_view;
                block_hash = prepared_block_hash;
                is_prepare = false;
            }

            if view != signed_commitment.commitment.view {
                return Err(EnclaveError::IncompatibleCommitmentForAccumulation);
            }

            if is_prepare {
                if block_view != signed_commitment.get_block_view() {
                    return Err(EnclaveError::IncompatibleCommitmentForAccumulation);
                }
                if block_hash != signed_commitment.get_block_hash() {
                    return Err(EnclaveError::IncompatibleCommitmentForAccumulation);
                }
            }

            if !self.verify_usig(signed_commitment)? {
                return Err(EnclaveError::IncompatibleCommitmentForAccumulation);
            }

            seen.insert(signed_commitment.commitment.creator);
            valid += 1;
            if valid >= self.quorum {
                break;
            }
        }

        if valid >= self.quorum {
            if is_prepare && view != block_view {
                return Err(EnclaveError::IncompatibleCommitmentForAccumulation);
            }
            self.create_commitment(
                view,
                block_view,
                block_hash.clone(),
                CommitmentType::Accumulator,
                is_prepare,
            )
        } else {
            Err(EnclaveError::NotEnoughValidCommitmentsForAccumulation)
        }
    }

    pub fn prepare(
        &mut self,
        block_view: View,
        block_hash: BlockHash,
        block_justification: Justification<Enc::Signature>,
        block_parent: Parent,
        justification_block_hash: BlockHash,
    ) -> Result<SignedCommitment<Enc::Signature>, EnclaveError<Enc::PlatformError>> {
        let Some(signed_commitment) = block_justification.get_commitment() else {
            return Err(EnclaveError::JustificationIsGenesis);
        };

        if signed_commitment.get_view_usable_for() != self.view {
            return Err(EnclaveError::JustificationInvalidView {
                justification_view: signed_commitment.get_view(),
                view_usable_for: signed_commitment.get_view_usable_for(),
                enclave_view: self.view,
                justification_block_view: signed_commitment.get_block_view(),
            });
        }

        if self.view != block_view {
            return Err(EnclaveError::BlockToPrepareIsForWrongView {
                enclave_view: self.view,
                block_view,
            });
        }

        if *signed_commitment.get_block_hash() != justification_block_hash {
            return Err(
                EnclaveError::JustificationBlockHashDoesNotMatchCommitmentBlockHash {
                    commitment_block_hash: signed_commitment.get_block_hash().clone(),
                    justification_block_hash,
                },
            );
        }

        if !self.verify_accumulator(signed_commitment)? {
            return Err(EnclaveError::InvalidAccumulatorSignature);
        }

        if block_parent == justification_block_hash {
            self.last_prepared_block_hash = signed_commitment.get_block_hash().clone();
            self.last_prepared_block_view = signed_commitment.get_block_view();
            debug!(
                "Block for view {} is now prepared",
                self.last_prepared_block_view
            );
        }

        self.create_usig(block_hash, self.view)
    }

    pub fn re_sign_last_prepared(
        &mut self,
    ) -> Result<SignedCommitment<Enc::Signature>, EnclaveError<Enc::PlatformError>> {
        self.create_usig(
            self.last_prepared_block_hash.clone(),
            self.last_prepared_block_view,
        )
    }

    fn create_usig(
        &mut self,
        block_hash: BlockHash,
        block_view: View,
    ) -> Result<SignedCommitment<Enc::Signature>, EnclaveError<Enc::PlatformError>> {
        let is_prepare = self.view == block_view;

        let commitment = self.create_commitment(
            self.view,
            block_view,
            block_hash,
            CommitmentType::Usig,
            is_prepare,
        );
        self.view.increment();
        commitment
    }

    fn create_commitment(
        &self,
        view: View,
        block_view: View,
        block_hash: BlockHash,
        commitment_type: CommitmentType,
        is_prepare: bool,
    ) -> Result<SignedCommitment<Enc::Signature>, EnclaveError<Enc::PlatformError>> {
        let commitment_phase = if is_prepare {
            CommitmentPhase::Prepare
        } else {
            CommitmentPhase::NewView
        };
        let commitment = Commitment {
            creator: self.me,
            view,
            block_hash,
            block_view,
            prepared_block_view: self.last_prepared_block_view,
            prepared_block_hash: self.last_prepared_block_hash.clone(),
            commitment_type,
            commitment_phase,
            precomputed_hash: OnceCell::new(),
        };
        let signature = self.enclave.sign(commitment.hash_value())?;
        Ok(SignedCommitment::new(commitment, signature))
    }

    fn verify_usig(
        &self,
        signed_commitment: &SignedCommitment<Enc::Signature>,
    ) -> Result<bool, EnclaveError<Enc::PlatformError>> {
        if !signed_commitment.commitment.is_usig() {
            Ok(false)
        } else {
            self.verify(signed_commitment)
        }
    }

    fn verify_accumulator(
        &self,
        signed_accumulator: &SignedCommitment<Enc::Signature>,
    ) -> Result<bool, EnclaveError<Enc::PlatformError>> {
        if !signed_accumulator.commitment.is_accumulator() {
            Ok(false)
        } else {
            self.verify(signed_accumulator)
        }
    }

    fn verify(
        &self,
        signed_commitment: &SignedCommitment<Enc::Signature>,
    ) -> Result<bool, EnclaveError<Enc::PlatformError>> {
        self.enclave.verify(
            signed_commitment.commitment.creator,
            signed_commitment.commitment.hash_value(),
            &signed_commitment.signature,
        )
    }
}
