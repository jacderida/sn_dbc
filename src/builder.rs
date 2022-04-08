// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use bls_ringct::{bls_bulletproofs::PedersenGens, group::Curve, ringct::Amount};
pub use bls_ringct::{
    ringct::RingCtTransaction, DecoyInput, MlsagMaterial, Output, RevealedCommitment,
    RingCtMaterial, TrueInput,
};
use blsttc::{PublicKey, SecretKey};
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::{
    rand::{CryptoRng, RngCore},
    AmountSecrets, Commitment, Dbc, DbcContent, Error, Hash, IndexedSignatureShare, KeyImage,
    KeyManager, OwnerOnce, Result, SpentProof, SpentProofContent, SpentProofShare,
    TransactionVerifier,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub type OutputOwnerMap = BTreeMap<PublicKey, OwnerOnce>;

/// A builder to create a RingCt transaction from
/// inputs and outputs.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug)]
pub struct TransactionBuilder {
    true_inputs: Vec<TrueInput>,
    ringct_material: RingCtMaterial,
    output_owner_map: OutputOwnerMap,
    available_decoys: Vec<DecoyInput>,
    decoys_per_input: usize,
    require_all_decoys: bool,
}

impl Default for TransactionBuilder {
    fn default() -> Self {
        Self {
            true_inputs: Default::default(),
            ringct_material: Default::default(),
            output_owner_map: Default::default(),
            available_decoys: Default::default(),
            decoys_per_input: 10, // default to 10 decoys per input.
            require_all_decoys: true,
        }
    }
}

impl TransactionBuilder {
    /// set decoys_per_input option.
    /// allocate this many decoys to each input (from available_decoys).
    pub fn set_decoys_per_input(mut self, decoys_per_input: usize) -> Self {
        self.decoys_per_input = decoys_per_input;
        self
    }

    /// set require_all_decoys option.
    /// if true, we require <decoys_per_input> for every (true) input.
    pub fn set_require_all_decoys(mut self, require_all_decoys: bool) -> Self {
        self.require_all_decoys = require_all_decoys;
        self
    }

    /// add to pool of available decoys.
    ///
    /// It is best that the size of the pool is larger (even much larger)
    /// than <decoys_per_input> * true_inputs.len().
    ///
    /// Especially because it is possible that 1 or more of these
    /// is actually a true input, which will not be discovered until
    /// ::build() is called.
    pub fn add_decoy_inputs(mut self, decoy_inputs: Vec<DecoyInput>) -> Self {
        let available_decoys = self.available_decoys.clone();
        let new_decoys = decoy_inputs.iter().filter(|new| {
            !available_decoys
                .iter()
                .any(|old| old.public_key() == new.public_key())
        });
        self.available_decoys.extend(new_decoys);
        self
    }

    /// add an input given an MlsagMaterial
    pub fn add_input(mut self, mlsag: MlsagMaterial) -> Self {
        // This requires a little explanation.
        //
        // We add the true input to our true_inputs list. This simplifies
        // the methods ::true_inputs() and ::inputs_owners() and
        // ::inputs_amount_sum(), which can operate on true_inputs alone.
        //
        // Finally in ::build(), any dups in true_inputs are removed,
        // and the remaining true_inputs are added to ringct_material.inputs.
        //
        // you asK: why not just put all inputs in ringct_material.inputs?
        //
        // answer:
        //   1. it would require an extra rng arg to all the other add_input*
        //      fns, which is ugly.
        //   2. it requires knowing the DecoyInputs in advance, which we can't
        //      really know until we know all the true inputs because we have
        //      to ensure that no true input is used as a decoy input.
        self = self.add_true_input(mlsag.true_input.clone());
        self.ringct_material.inputs.push(mlsag);
        self
    }

    /// add an input given an iterator over MlsagMaterial
    pub fn add_inputs(mut self, inputs: impl IntoIterator<Item = MlsagMaterial>) -> Self {
        self.ringct_material.inputs.extend(inputs);
        self
    }

    /// add an input given a TrueInput
    pub fn add_true_input(mut self, true_input: TrueInput) -> Self {
        self.true_inputs.push(true_input);
        self
    }

    /// add an input given a list of TrueInputs and associated decoys
    pub fn add_true_inputs(mut self, inputs: impl IntoIterator<Item = TrueInput>) -> Self {
        for true_input in inputs.into_iter() {
            self = self.add_true_input(true_input);
        }
        self
    }

    /// add an input given a Dbc, SecretKey and decoy list
    pub fn add_input_dbc(mut self, dbc: &Dbc, base_sk: &SecretKey) -> Result<Self> {
        self = self.add_true_input(dbc.as_true_input(base_sk)?);
        Ok(self)
    }

    /// add an input given a list of Dbcs and associated SecretKey and decoys
    pub fn add_inputs_dbc(
        mut self,
        dbcs: impl IntoIterator<Item = (Dbc, SecretKey)>,
    ) -> Result<Self> {
        for (dbc, base_sk) in dbcs.into_iter() {
            self = self.add_input_dbc(&dbc, &base_sk)?;
        }
        Ok(self)
    }

    /// add an input given a bearer Dbc, SecretKey and decoy list
    pub fn add_input_dbc_bearer(mut self, dbc: &Dbc) -> Result<Self> {
        self = self.add_true_input(dbc.as_true_input_bearer()?);
        Ok(self)
    }

    /// add an input given a list of bearer Dbcs and associated SecretKey and decoys
    pub fn add_inputs_dbc_bearer(mut self, dbcs: impl IntoIterator<Item = Dbc>) -> Result<Self> {
        for dbc in dbcs.into_iter() {
            self = self.add_input_dbc_bearer(&dbc)?;
        }
        Ok(self)
    }

    /// add an input given a SecretKey, AmountSecrets, and list of decoys
    pub fn add_input_by_secrets(
        mut self,
        secret_key: SecretKey,
        amount_secrets: AmountSecrets,
    ) -> Self {
        let true_input = TrueInput::new(secret_key, amount_secrets.into());
        self = self.add_true_input(true_input);
        self
    }

    /// add an input given a list of (SecretKey, AmountSecrets, and list of decoys)
    pub fn add_inputs_by_secrets(mut self, secrets: Vec<(SecretKey, AmountSecrets)>) -> Self {
        for (secret_key, amount_secrets) in secrets.into_iter() {
            self = self.add_input_by_secrets(secret_key, amount_secrets);
        }
        self
    }

    /// add an output
    pub fn add_output(mut self, output: Output, owner: OwnerOnce) -> Self {
        self.output_owner_map
            .insert(output.public_key().into(), owner);
        self.ringct_material.outputs.push(output);
        self
    }

    /// add a list of outputs
    pub fn add_outputs(mut self, outputs: impl IntoIterator<Item = (Output, OwnerOnce)>) -> Self {
        for (output, owner) in outputs.into_iter() {
            self = self.add_output(output, owner);
        }
        self
    }

    /// add an output by providing Amount and OwnerOnce
    pub fn add_output_by_amount(mut self, amount: Amount, owner: OwnerOnce) -> Self {
        let pk = owner.as_owner().public_key();
        let output = Output::new(pk, amount);
        self.output_owner_map.insert(pk, owner);
        self.ringct_material.outputs.push(output);
        self
    }

    /// add an output by providing iter of (Amount, OwnerOnce)
    pub fn add_outputs_by_amount(
        mut self,
        outputs: impl IntoIterator<Item = (Amount, OwnerOnce)>,
    ) -> Self {
        for (amount, owner) in outputs.into_iter() {
            self = self.add_output_by_amount(amount, owner);
        }
        self
    }

    /// get a list of input (true) owners
    pub fn input_owners(&self) -> Vec<PublicKey> {
        self.true_inputs
            .iter()
            .map(|t| t.public_key().into())
            .collect()
    }

    /// get sum of input amounts
    pub fn inputs_amount_sum(&self) -> Amount {
        self.true_inputs
            .iter()
            .map(|t| t.revealed_commitment.value)
            .sum()
    }

    /// get sum of output amounts
    pub fn outputs_amount_sum(&self) -> Amount {
        self.ringct_material.outputs.iter().map(|o| o.amount).sum()
    }

    /// get true inputs
    pub fn inputs(&self) -> &Vec<TrueInput> {
        &self.true_inputs
    }

    /// get outputs
    pub fn outputs(&self) -> &Vec<Output> {
        &self.ringct_material.outputs
    }

    /// build a RingCtTransaction and associated secrets
    pub fn build(self, mut rng: impl RngCore + CryptoRng) -> Result<DbcBuilder> {
        let mut ringct_material = self.ringct_material;
        let mut true_inputs = self.true_inputs;

        // get public_keys of all true_inputs.
        let true_public_keys: Vec<_> = true_inputs
            .iter()
            .map(|true_input| true_input.public_key().to_affine())
            .collect();
        // remove any available decoys that are actually true inputs.
        let available_decoys: Vec<_> = self
            .available_decoys
            .into_iter()
            .filter(|d| true_public_keys.iter().all(|pk| *pk != d.public_key()))
            .collect();

        // remove any true inputs that are already in self.ringct_material.inputs
        // see comment in ::add_input() for explanation.
        true_inputs.retain(|true_input| {
            !ringct_material
                .inputs
                .iter()
                .any(|m| m.true_input.public_key() == true_input.public_key())
        });

        // calc total number of decoys required for Tx.
        let num_required_decoys = true_inputs.len() * self.decoys_per_input;
        if self.require_all_decoys && available_decoys.len() < num_required_decoys {
            return Err(Error::InsufficientDecoys);
        }

        // group available decoys into sets of <decoys_per_input>.
        let mut decoy_inputs_chunks: Vec<&[DecoyInput]> = match self.decoys_per_input {
            0 => vec![], // ::chunks() panics if chunk-size is zero.
            _ => available_decoys.chunks(self.decoys_per_input).collect(),
        };

        // if we don't have enough sets of decoys, then we need to add any
        // missing sets, to match true_inputs.len()
        let empty: Vec<DecoyInput> = vec![];
        if decoy_inputs_chunks.len() < true_inputs.len() {
            assert!(!self.require_all_decoys);
            assert!(num_required_decoys == 0 || num_required_decoys > available_decoys.len());

            // pad to true_inputs.len with empty vec(s).
            while decoy_inputs_chunks.len() < true_inputs.len() {
                decoy_inputs_chunks.push(&empty);
            }
        }

        // create our final ringct inputs, with decoys.
        for (true_input, decoy_inputs) in true_inputs.into_iter().zip(decoy_inputs_chunks) {
            ringct_material.inputs.push(MlsagMaterial::new(
                true_input,
                decoy_inputs.to_vec(),
                &mut rng,
            ));
        }

        // Grand finale!  sign the ringct_material to generate a Tx.
        let result: Result<(RingCtTransaction, Vec<RevealedCommitment>)> =
            ringct_material.sign(rng).map_err(|e| e.into());
        let (transaction, revealed_commitments) = result?;

        Ok(DbcBuilder::new(
            transaction,
            revealed_commitments,
            self.output_owner_map,
            ringct_material,
        ))
    }
}

/// A Builder for aggregating SpentProofs and generating the final Dbc outputs.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct DbcBuilder {
    pub transaction: RingCtTransaction,
    pub revealed_commitments: Vec<RevealedCommitment>,
    pub output_owner_map: OutputOwnerMap,
    pub ringct_material: RingCtMaterial,

    pub spent_proof_shares: BTreeMap<KeyImage, HashSet<SpentProofShare>>,
}

impl DbcBuilder {
    /// Create a new DbcBuilder
    pub fn new(
        transaction: RingCtTransaction,
        revealed_commitments: Vec<RevealedCommitment>,
        output_owner_map: OutputOwnerMap,
        ringct_material: RingCtMaterial,
    ) -> Self {
        Self {
            transaction,
            revealed_commitments,
            output_owner_map,
            spent_proof_shares: Default::default(),
            ringct_material,
        }
    }

    /// returns Vec of key_image and tx intended for use as inputs
    /// to Spendbook::log_spent().
    pub fn inputs(&self) -> Vec<(KeyImage, RingCtTransaction)> {
        self.transaction
            .mlsags
            .iter()
            .map(|mlsag| (mlsag.key_image.into(), self.transaction.clone()))
            .collect()
    }

    /// Add a SpentProofShare for the given input index
    pub fn add_spent_proof_share(mut self, share: SpentProofShare) -> Self {
        let shares = self
            .spent_proof_shares
            .entry(*share.key_image())
            .or_default();
        shares.insert(share);
        self
    }

    /// Add a list of SpentProofShare for the given input index
    pub fn add_spent_proof_shares(
        mut self,
        shares: impl IntoIterator<Item = SpentProofShare>,
    ) -> Self {
        for share in shares.into_iter() {
            self = self.add_spent_proof_share(share);
        }
        self
    }

    /// Build the output DBCs
    ///
    /// see TransactionVerifier::verify() for a description of
    /// verifier requirements.
    pub fn build<K: KeyManager>(
        self,
        verifier: &K,
    ) -> Result<Vec<(Dbc, OwnerOnce, AmountSecrets)>> {
        let spent_proofs = self.spent_proofs()?;

        // verify the Tx, along with spent proofs.
        // note that we do this just once for entire Tx, not once per output Dbc.
        TransactionVerifier::verify(verifier, &self.transaction, &spent_proofs)?;

        let pc_gens = PedersenGens::default();
        let output_commitments: Vec<(Commitment, RevealedCommitment)> = self
            .revealed_commitments
            .iter()
            .map(|r| (r.commit(&pc_gens).to_affine(), *r))
            .collect();

        let owner_once_list: Vec<&OwnerOnce> = self
            .transaction
            .outputs
            .iter()
            .map(|output| {
                self.output_owner_map
                    .get(&(*output.public_key()).into())
                    .ok_or(Error::PublicKeyNotFound)
            })
            .collect::<Result<_>>()?;

        // Form the final output DBCs
        let output_dbcs: Vec<(Dbc, OwnerOnce, AmountSecrets)> = self
            .transaction
            .outputs
            .iter()
            .zip(owner_once_list)
            .map(|(output, owner_once)| {
                let amount_secrets_list: Vec<AmountSecrets> = output_commitments
                    .iter()
                    .filter(|(c, _)| *c == output.commitment())
                    .map(|(_, r)| AmountSecrets::from(*r))
                    .collect();
                assert_eq!(amount_secrets_list.len(), 1);

                let dbc = Dbc {
                    content: DbcContent::from((
                        owner_once.owner_base.clone(),
                        owner_once.derivation_index,
                        amount_secrets_list[0].clone(),
                    )),
                    transaction: self.transaction.clone(),
                    spent_proofs: spent_proofs.clone(),
                };
                (dbc, owner_once.clone(), amount_secrets_list[0].clone())
            })
            .collect();

        Ok(output_dbcs)
    }

    /// build spent proofs from shares.
    pub fn spent_proofs(&self) -> Result<BTreeSet<SpentProof>> {
        let spent_proofs: BTreeSet<SpentProof> = self
            .spent_proof_shares
            .iter()
            .map(|(key_image, shares)| {
                let any_share = shares
                    .iter()
                    .next()
                    .ok_or(Error::MissingSpentProofShare(*key_image))?;

                let spentbook_pub_key = any_share.spentbook_pks().public_key();
                let spentbook_sig = any_share.spentbook_pks.combine_signatures(
                    shares
                        .iter()
                        .map(SpentProofShare::spentbook_sig_share)
                        .map(IndexedSignatureShare::threshold_crypto),
                )?;

                let public_commitments: Vec<Commitment> = any_share.public_commitments().clone();

                let spent_proof = SpentProof {
                    content: SpentProofContent {
                        key_image: *key_image,
                        transaction_hash: Hash::from(self.transaction.hash()),
                        public_commitments,
                    },
                    spentbook_pub_key,
                    spentbook_sig,
                };

                Ok(spent_proof)
            })
            .collect::<Result<_>>()?;

        Ok(spent_proofs)
    }
}
