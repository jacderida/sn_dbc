// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! This module defines blstrs aliases
//!
//! We provide type aliases to make the usage in each context clearer and to make the
//! the sn_dbc public API simpler so that the caller should not need to depend on blstrs
//! and use its types directly.
//!
//! sn_dbc internally uses the type aliases rather than directly using the blstrs types.
//!
//! We could consider moving some or all of this lower into sn_ringct to make these
//! crates consistent.

/// a Commitment
pub type Commitment = bls_ringct::blstrs::G1Affine;

/// a BlindingFactor
pub type BlindingFactor = bls_ringct::blstrs::Scalar;

/// A KeyImage can be thought of as a specific type
/// of public key. blsttc::PublicKey is a newtype
/// wrapper around blstrs::G1Affine.  We use
/// PublicKey because it impls Hash and Ord traits
/// that are useful for storing the KeyImage in
/// a map.
pub type KeyImage = blsttc::PublicKey;
