use std::convert::Infallible;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::ops::Deref;

use blake2::Blake2b512;
use hashbar::Hashbar;
use heaps::Keyable;
use id_set::ReplicaSet;
use serde::de::{Error, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::digest::Update;
use sha2::Digest;
use sha2::Sha256;
use shared_ids::ReplicaId;

use crate::DagAddress;
use crate::Round;

#[derive(Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
#[repr(transparent)]
#[serde(transparent)]
struct TransactionsHash([u8; 32]);

impl<T: Into<[u8; 32]>> From<T> for TransactionsHash {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl AsRef<[u8]> for TransactionsHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Debug for TransactionsHash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TransactionsHash({})", hex::encode(self.0))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SignableVertex {
    address: DagAddress,
    transactions_hash: TransactionsHash,
    edges: ReplicaSet,
}

impl Hashbar for SignableVertex {
    fn hash<H: Update>(&self, hasher: &mut H) {
        hasher.update(&self.round().as_u64().to_le_bytes());
        hasher.update(&self.creator().as_u64().to_le_bytes());
        hasher.update(self.transactions_hash.as_ref());
        for edge in self.edges.iter() {
            hasher.update(&edge.as_u64().to_le_bytes());
        }
    }
}

impl Deref for SignableVertex {
    type Target = DagAddress;

    fn deref(&self) -> &Self::Target {
        &self.address
    }
}

impl SignableVertex {
    fn new<Tx: Hashbar>(address: DagAddress, edges: ReplicaSet, transactions: &[Tx]) -> Self {
        let mut hasher = Sha256::new();
        for tx in transactions {
            tx.hash(&mut hasher);
        }
        let transactions_hash = hasher.finalize().into();
        Self {
            address,
            edges,
            transactions_hash,
        }
    }

    pub fn address(&self) -> DagAddress {
        self.address
    }

    pub fn edges(&self) -> &ReplicaSet {
        &self.edges
    }

    pub fn num_edges(&self) -> u64 {
        self.edges.len()
    }

    pub fn edges_valid(&self, quorum: u64) -> bool {
        self.num_edges() >= quorum
    }

    pub fn to_hash(&self) -> impl AsRef<[u8]> {
        let mut hasher = Blake2b512::new();

        Hashbar::hash(self, &mut hasher);

        hasher.finalize()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SignedVertex<Sig> {
    signable_vertex: SignableVertex,
    signature: Sig,
    counter: u64,
}

impl<Sig> Deref for SignedVertex<Sig> {
    type Target = SignableVertex;

    fn deref(&self) -> &Self::Target {
        &self.signable_vertex
    }
}

impl<Sig> SignedVertex<Sig> {
    fn new(signable_vertex: SignableVertex, signature: Sig, counter: u64) -> Self {
        Self {
            signable_vertex,
            signature,
            counter,
        }
    }

    pub fn signature(&self) -> &Sig {
        &self.signature
    }

    pub fn counter(&self) -> u64 {
        self.counter
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Vertex<Tx, Sig> {
    transactions: Vec<Tx>,
    signed_vertex: SignedVertex<Sig>,
}

impl<Tx, Sig> Deref for Vertex<Tx, Sig> {
    type Target = SignedVertex<Sig>;

    fn deref(&self) -> &Self::Target {
        &self.signed_vertex
    }
}

impl<Tx: Hashbar, Sig> Vertex<Tx, Sig> {
    pub fn new<E>(
        round: Round,
        creator: ReplicaId,
        edges: ReplicaSet,
        transactions: Vec<Tx>,
        sign: impl FnOnce(&SignableVertex) -> Result<(Sig, u64), E>,
    ) -> Result<Self, E> {
        Self::new_with_custom(round, creator, edges, transactions, |v| {
            sign(v).map(|(a, b)| (a, b, ()))
        })
        .map(|(s, ())| s)
    }

    pub fn new_with_custom<E, C>(
        round: Round,
        creator: ReplicaId,
        edges: ReplicaSet,
        transactions: Vec<Tx>,
        sign: impl FnOnce(&SignableVertex) -> Result<(Sig, u64, C), E>,
    ) -> Result<(Self, C), E> {
        let address = DagAddress::new(round, creator);
        let signable_vertex = SignableVertex::new(address, edges, &transactions);

        let (signature, counter, custom) = sign(&signable_vertex)?;

        let signed_vertex = SignedVertex::new(signable_vertex, signature, counter);
        Ok((
            Self {
                transactions,
                signed_vertex,
            },
            custom,
        ))
    }

    fn new_with_signature(
        round: Round,
        creator: ReplicaId,
        edges: ReplicaSet,
        transactions: Vec<Tx>,
        signature: Sig,
        counter: u64,
    ) -> Self {
        Self::new(round, creator, edges, transactions, |_| {
            Ok::<_, Infallible>((signature, counter))
        })
        .unwrap_or_else(|e| match e {})
    }

    pub fn transactions(&self) -> &Vec<Tx> {
        &self.transactions
    }

    pub fn drain(self) -> Vec<Tx> {
        self.transactions
    }
}

impl<Tx, Sig> Keyable for Vertex<Tx, Sig> {
    type Key = DagAddress;
    fn key(&self) -> Self::Key {
        self.address()
    }
}

impl<Tx: Serialize, Sig: Serialize> Serialize for Vertex<Tx, Sig> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut struct_serializer = serializer.serialize_struct("Vertex", 6)?;
        struct_serializer.serialize_field("round", &self.round)?;
        struct_serializer.serialize_field("creator", &self.creator)?;
        struct_serializer.serialize_field("edges", &self.edges)?;
        struct_serializer.serialize_field("transactions", &self.transactions)?;
        struct_serializer.serialize_field("signature", &self.signature)?;
        struct_serializer.serialize_field("counter", &self.counter)?;
        struct_serializer.end()
    }
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum VertexVisitorField {
    Round,
    Creator,
    Edges,
    Transactions,
    Signature,
    Counter,
}

struct VertexVisitor<Tx, Sig> {
    phantom_data: PhantomData<(Tx, Sig)>,
}

impl<'de, Tx: Deserialize<'de> + Hashbar, Sig: Deserialize<'de>> Visitor<'de>
    for VertexVisitor<Tx, Sig>
{
    type Value = Vertex<Tx, Sig>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "struct Vertex")?;
        Ok(())
    }

    #[inline]
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        const EXPECTED: &str = "struct Vertex with 6 elements";
        let round = seq
            .next_element::<Round>()?
            .ok_or_else(|| Error::invalid_length(0, &EXPECTED))?;

        let creator = seq
            .next_element::<ReplicaId>()?
            .ok_or_else(|| Error::invalid_length(1, &EXPECTED))?;

        let edges = seq
            .next_element::<ReplicaSet>()?
            .ok_or_else(|| Error::invalid_length(2, &EXPECTED))?;

        let transactions = seq
            .next_element::<Vec<Tx>>()?
            .ok_or_else(|| Error::invalid_length(3, &EXPECTED))?;

        let signature = seq
            .next_element::<Sig>()?
            .ok_or_else(|| Error::invalid_length(4, &EXPECTED))?;

        let counter = seq
            .next_element::<u64>()?
            .ok_or_else(|| Error::invalid_length(5, &EXPECTED))?;

        Ok(Vertex::new_with_signature(
            round,
            creator,
            edges,
            transactions,
            signature,
            counter,
        ))
    }

    #[inline]
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut round: Option<Round> = None;
        let mut creator: Option<ReplicaId> = None;
        let mut edges: Option<ReplicaSet> = None;
        let mut transactions: Option<Vec<Tx>> = None;
        let mut signature: Option<Sig> = None;
        let mut counter: Option<u64> = None;
        while let Some(key) = map.next_key::<VertexVisitorField>()? {
            match key {
                VertexVisitorField::Round => {
                    if round.is_some() {
                        return Err(Error::duplicate_field("round"));
                    }
                    round = Some(map.next_value::<Round>()?);
                }
                VertexVisitorField::Creator => {
                    if creator.is_some() {
                        return Err(Error::duplicate_field("creator"));
                    }
                    creator = Some(map.next_value::<ReplicaId>()?);
                }
                VertexVisitorField::Edges => {
                    if edges.is_some() {
                        return Err(Error::duplicate_field("edges"));
                    }
                    edges = Some(map.next_value::<ReplicaSet>()?);
                }
                VertexVisitorField::Transactions => {
                    if transactions.is_some() {
                        return Err(Error::duplicate_field("transactions"));
                    }
                    transactions = Some(map.next_value::<Vec<Tx>>()?);
                }
                VertexVisitorField::Signature => {
                    if signature.is_some() {
                        return Err(Error::duplicate_field("signature"));
                    }
                    signature = Some(map.next_value::<Sig>()?);
                }
                VertexVisitorField::Counter => {
                    if counter.is_some() {
                        return Err(Error::duplicate_field("counter"));
                    }
                    counter = Some(map.next_value::<u64>()?);
                }
            }
        }

        let round = round.ok_or_else(|| Error::missing_field("round"))?;
        let creator = creator.ok_or_else(|| Error::missing_field("creator"))?;
        let edges = edges.ok_or_else(|| Error::missing_field("edges"))?;
        let transactions = transactions.ok_or_else(|| Error::missing_field("transactions"))?;
        let signature = signature.ok_or_else(|| Error::missing_field("signature"))?;
        let counter = counter.ok_or_else(|| Error::missing_field("counter"))?;

        Ok(Vertex::new_with_signature(
            round,
            creator,
            edges,
            transactions,
            signature,
            counter,
        ))
    }
}

impl<'de, Tx: Deserialize<'de> + Hashbar, Sig: Deserialize<'de>> Deserialize<'de>
    for Vertex<Tx, Sig>
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_struct(
            "Vertex",
            &[
                "round",
                "creator",
                "edges",
                "transactions",
                "signature",
                "counter",
            ],
            VertexVisitor {
                phantom_data: PhantomData,
            },
        )
    }
}
