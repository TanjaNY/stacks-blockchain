/*
 copyright: (c) 2013-2019 by Blockstack PBC, a public benefit corporation.

 This file is part of Blockstack.

 Blockstack is free software. You may redistribute or modify
 it under the terms of the GNU General Public License as published by
 the Free Software Foundation, either version 3 of the License or
 (at your option) any later version.

 Blockstack is distributed in the hope that it will be useful,
 but WITHOUT ANY WARRANTY, including without the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 GNU General Public License for more details.

 You should have received a copy of the GNU General Public License
 along with Blockstack. If not, see <http://www.gnu.org/licenses/>.
*/

use chainstate::stacks::*;
use chainstate::stacks::index::TrieHash;

use chainstate::burn::BlockHeaderHash;

use net::StacksMessageCodec;
use net::Error as net_error;
use net::codec::{read_next, write_next};

use util::hash::MerkleTree;
use util::hash::Sha512Trunc256Sum;
use util::secp256k1::MessageSignature;

use net::StacksPublicKeyBuffer;

use sha2::Sha512Trunc256;
use sha2::Digest;

use chainstate::burn::*;
use chainstate::burn::operations::*;

use burnchains::BurnchainHeaderHash;
use burnchains::PrivateKey;
use burnchains::PublicKey;

use util::vrf::*;

impl StacksMessageCodec for VRFProof {
    fn serialize(&self) -> Vec<u8> {
        self.to_bytes().to_vec()
    }

    fn deserialize(buf: &Vec<u8>, index_ptr: &mut u32, max_size: u32) -> Result<VRFProof, net_error> {
        let index = *index_ptr;
        if index > u32::max_value() - VRF_PROOF_ENCODED_SIZE {
            return Err(net_error::OverflowError);
        }
        if index + VRF_PROOF_ENCODED_SIZE > max_size {
            return Err(net_error::OverflowError);
        }
        if (buf.len() as u32) < index + VRF_PROOF_ENCODED_SIZE {
            return Err(net_error::UnderflowError);
        }

        let res = VRFProof::from_slice(&buf[(index as usize)..((index+VRF_PROOF_ENCODED_SIZE) as usize)])
            .ok_or(net_error::DeserializeError)?;
            
        *index_ptr += VRF_PROOF_ENCODED_SIZE;
        Ok(res)
    }
}

impl StacksMessageCodec for StacksWorkScore {
    fn serialize(&self) -> Vec<u8> {
        let mut ret = vec![];
        write_next(&mut ret, &self.burn);
        write_next(&mut ret, &self.work);
        ret
    }

    fn deserialize(buf: &Vec<u8>, index_ptr: &mut u32, max_size: u32) -> Result<StacksWorkScore, net_error> {
        let mut index = *index_ptr;
        let burn = read_next(buf, &mut index, max_size)?;
        let work = read_next(buf, &mut index, max_size)?;

        *index_ptr = index;
        Ok(StacksWorkScore {
            burn,
            work
        })
    }
}

impl StacksWorkScore {
    pub fn initial() -> StacksWorkScore {
        StacksWorkScore {
            burn: 0,
            work: 0
        }
    }

    pub fn add(&self, work_delta: &StacksWorkScore) -> StacksWorkScore {
        let mut ret = self.clone();
        ret.burn = self.burn.checked_add(work_delta.burn).expect("FATAL: numeric overflow on calculating new total burn");
        ret.work = self.work.checked_add(work_delta.work).expect("FATAL: numeric overflow on calculating new total work");
        ret
    }
}

impl StacksMessageCodec for StacksBlockHeader {
    fn serialize(&self) -> Vec<u8> {
        let mut ret = vec![];
        write_next(&mut ret, &self.version);
        write_next(&mut ret, &self.total_work);
        write_next(&mut ret, &self.proof);
        write_next(&mut ret, &self.parent_block);
        write_next(&mut ret, &self.parent_microblock);
        write_next(&mut ret, &self.parent_microblock_sequence);
        write_next(&mut ret, &self.tx_merkle_root);
        write_next(&mut ret, &self.state_index_root);
        write_next(&mut ret, &self.microblock_pubkey_hash);
        ret
    }

    fn deserialize(buf: &Vec<u8>, index_ptr: &mut u32, max_size: u32) -> Result<StacksBlockHeader, net_error> {
        let mut index = *index_ptr;

        let version: u8                         = read_next(buf, &mut index, max_size)?;
        let total_work : StacksWorkScore        = read_next(buf, &mut index, max_size)?;
        let proof : VRFProof                    = read_next(buf, &mut index, max_size)?;
        let parent_block: BlockHeaderHash       = read_next(buf, &mut index, max_size)?;
        let parent_microblock: BlockHeaderHash  = read_next(buf, &mut index, max_size)?;
        let parent_microblock_sequence: u8      = read_next(buf, &mut index, max_size)?;
        let tx_merkle_root: Sha512Trunc256Sum   = read_next(buf, &mut index, max_size)?;
        let state_index_root: TrieHash          = read_next(buf, &mut index, max_size)?;
        let pubkey_hash_buf: Hash160            = read_next(buf, &mut index, max_size)?;

        *index_ptr = index;
        Ok(StacksBlockHeader {
            version,
            total_work,
            proof,
            parent_block,
            parent_microblock,
            parent_microblock_sequence,
            tx_merkle_root,
            state_index_root,
            microblock_pubkey_hash: pubkey_hash_buf
        })
    }
}

impl StacksBlockHeader {
    pub fn pubkey_hash(pubk: &StacksPublicKey) -> Hash160 {
        let pubkey_buf = StacksPublicKeyBuffer::from_public_key(pubk);
        let bytes = pubkey_buf.serialize();
        Hash160::from_data(&bytes[..])
    }
    
    pub fn genesis() -> StacksBlockHeader {
        StacksBlockHeader {
            version: STACKS_BLOCK_VERSION,
            total_work: StacksWorkScore::initial(),
            proof: VRFProof::empty(),
            parent_block: BlockHeaderHash([0u8; 32]),
            parent_microblock: BlockHeaderHash([0u8; 32]),
            parent_microblock_sequence: 0,
            tx_merkle_root: Sha512Trunc256Sum([0u8; 32]),
            state_index_root: TrieHash([0u8; 32]),
            microblock_pubkey_hash: Hash160([0u8; 20]),
        }
    }

    pub fn is_genesis(&self) -> bool {
        self.parent_block == BlockHeaderHash([0u8; 32])
    }

    pub fn block_hash(&self) -> BlockHeaderHash {
        let buf = self.serialize();
        BlockHeaderHash::from_serialized_header(&buf[..])
    }

    /// This is the "block hash" used for extending the state index root.
    /// This method is necessary because the index root must be globally unique (but, the same stacks
    /// block header can show up multiple times on different burn chain forks).
    pub fn make_index_block_hash(burn_block_hash: &BurnchainHeaderHash, block_hash: &BlockHeaderHash) -> BlockHeaderHash {
        let mut hash_bytes = vec![];
        hash_bytes.extend_from_slice(&mut block_hash.as_bytes().clone());
        hash_bytes.extend_from_slice(&mut burn_block_hash.as_bytes().clone());
        
        let h = Sha512Trunc256Sum::from_data(&hash_bytes);
        let mut b = [0u8; 32];
        b.copy_from_slice(h.as_bytes());
        BlockHeaderHash(b)
    }

    pub fn index_block_hash(&self, burn_hash: &BurnchainHeaderHash) -> BlockHeaderHash {
        let block_hash = self.block_hash();
        StacksBlockHeader::make_index_block_hash(burn_hash, &block_hash)
    }

    pub fn from_parent(parent_header: &StacksBlockHeader,
                       parent_microblock_header: Option<&StacksMicroblockHeader>,
                       total_work: &StacksWorkScore,
                       proof: &VRFProof,
                       tx_merkle_root: &Sha512Trunc256Sum,
                       state_index_root: &TrieHash,
                       microblock_pubkey_hash: &Hash160) -> StacksBlockHeader {

        let (parent_microblock, parent_microblock_sequence) = match parent_microblock_header {
            Some(header) => {
                (header.block_hash(), header.sequence)
            },
            None => {
                (BlockHeaderHash([0u8; 32]), 0)
            }
        };

        StacksBlockHeader {
            version: STACKS_BLOCK_VERSION,
            total_work: total_work.clone(),
            proof: proof.clone(),
            parent_block: parent_header.block_hash(),
            parent_microblock: parent_microblock,
            parent_microblock_sequence: parent_microblock_sequence,
            tx_merkle_root: tx_merkle_root.clone(),
            state_index_root: state_index_root.clone(),
            microblock_pubkey_hash: microblock_pubkey_hash.clone(),
        }
    }

    pub fn from_parent_empty(parent_header: &StacksBlockHeader, parent_microblock_header: Option<&StacksMicroblockHeader>, work_delta: &StacksWorkScore, proof: &VRFProof, microblock_pubkey_hash: &Hash160) -> StacksBlockHeader {
        StacksBlockHeader::from_parent(parent_header, parent_microblock_header, work_delta, proof, &Sha512Trunc256Sum([0u8; 32]), &TrieHash([0u8; 32]), microblock_pubkey_hash)
    }

    /// Validate this block header against the burnchain.
    /// Used to determine whether or not we'll keep a block around (even if we don't yet have its parent).
    /// * burn_chain_tip is the BlockSnapshot encoding the sortition that selected this block for
    /// inclusion in the Stacks blockchain chain state.
    /// * stacks_chain_tip is the BlockSnapshot for the parent Stacks block this header builds on
    /// (i.e. this is the BlockSnapshot that corresponds to the parent of the given block_commit).
    pub fn validate_burnchain(&self, burn_chain_tip: &BlockSnapshot, sortition_chain_tip: &BlockSnapshot, leader_key: &LeaderKeyRegisterOp, block_commit: &LeaderBlockCommitOp, stacks_chain_tip: &BlockSnapshot) -> bool {
        // the burn chain tip's sortition must have chosen given block commit
        assert_eq!(burn_chain_tip.winning_stacks_block_hash, block_commit.block_header_hash);
        assert_eq!(burn_chain_tip.winning_block_txid, block_commit.txid);
        
        // this header must match the header that won sortition on the burn chain
        if self.block_hash() != burn_chain_tip.winning_stacks_block_hash {
            debug!("Invalid Stacks block header {}: invalid commit: {} != {}", self.block_hash().to_hex(), self.block_hash().to_hex(), burn_chain_tip.winning_stacks_block_hash.to_hex());
            return false;
        }

        // this header must match the parent header as recorded on the burn chain
        if self.parent_block != stacks_chain_tip.winning_stacks_block_hash {
            debug!("Invalid Stacks block header {}: invalid parent hash: {} != {}", self.block_hash().to_hex(), self.parent_block.to_hex(), stacks_chain_tip.winning_stacks_block_hash.to_hex());
            return false;
        }
        
        // this header's proof must hash to the burn chain tip's VRF seed
        if !block_commit.new_seed.is_from_proof(&self.proof) {
            debug!("Invalid Stacks block header {}: invalid VRF proof: hash({}) != {} (but {})", self.block_hash().to_hex(), self.proof.to_hex(), block_commit.new_seed.to_hex(), VRFSeed::from_proof(&self.proof).to_hex());
            return false;
        }

        // this header must commit to all of the work seen so far in this stacks blockchain fork.
        if self.total_work.burn != stacks_chain_tip.total_burn {
            debug!("Invalid Stacks block header {}: invalid total burns: {} != {}", self.block_hash().to_hex(), self.total_work.burn, stacks_chain_tip.total_burn);
            return false;
        }

        // TODO: work score?

        // this header's VRF proof must have been generated from the last sortition's sortition
        // hash (which includes the last commit's VRF seed)
        let valid = match VRF::verify(&leader_key.public_key, &self.proof, &sortition_chain_tip.sortition_hash.as_bytes().to_vec()) {
            Ok(v) => {
                v
            },
            Err(e) => {
                false
            }
        };

        if !valid {
            debug!("Invalid Stacks block header {}: leader VRF key {} did not produce a valid proof over {}", self.block_hash().to_hex(), leader_key.public_key.to_hex(), burn_chain_tip.sortition_hash.to_hex());
            return false;
        }

        // not verified by this method:
        // * parent_microblock and parent_microblock_sequence
        // use validate_parent() for that.
        //
        // also not verified:
        // * tx_merkle_root     (already verified; validated on deserialization)
        // * state_index_root   (validated on process_block())
        true
    }

    /// Validate this block header against its parent block header.
    /// Call after validate_burnchain()
    pub fn validate_parent(&self, parent: &StacksBlockHeader, microblock_parent_opt: Option<&StacksMicroblockHeader>) -> bool {
        if parent.block_hash() != self.parent_block {
            debug!("Invalid Stacks block header {}: parent {} != {}", self.block_hash().to_hex(), parent.block_hash().to_hex(), self.parent_block.to_hex());
            return false;
        }

        if let Some(ref microblock_parent) = microblock_parent_opt {
            if self.parent_microblock_sequence == 0 {
                // we have no parent microblock 
                debug!("Invalid Stacks block header {}: no parent microblock (sequence 0), but expected {}", self.block_hash().to_hex(), microblock_parent.sequence);
                return false;
            }

            if microblock_parent.block_hash() != self.parent_microblock {
                debug!("Invalid Stacks block header {}: parent microblock {} != {}", self.block_hash().to_hex(), microblock_parent.block_hash().to_hex(), self.parent_microblock.to_hex());
                return false;
            }

            if microblock_parent.sequence != self.parent_microblock_sequence {
                debug!("Invalid Stacks block header {}: parent microblock sequence {} != {}", self.block_hash().to_hex(), microblock_parent.sequence, self.parent_microblock_sequence);
                return false;
            }
        }
        else {
            // no parent microblock, so these fields must be 0'ed
            if self.parent_microblock_sequence != 0 {
                debug!("Invalid Stacks block header {}: sequence is not 0 (but is {})", self.block_hash().to_hex(), self.parent_microblock_sequence);
                return false;
            }

            if self.parent_microblock != BlockHeaderHash([0u8; 32]) {
                debug!("Invalid Stacks block header {}: parent microblock {} != {}", self.block_hash().to_hex(), self.parent_microblock.to_hex(), BlockHeaderHash([0u8; 32]).to_hex());
                return false;
            }
        }

        return true;
    }
}

impl StacksMessageCodec for StacksBlock {
    fn serialize(&self) -> Vec<u8> {
        let mut ret = vec![];
        write_next(&mut ret, &self.header);
        write_next(&mut ret, &self.txs);
        ret
    }

    fn deserialize(buf: &Vec<u8>, index_ptr: &mut u32, max_size: u32) -> Result<StacksBlock, net_error> {
        // no matter what, do not allow us to parse a block bigger than the epoch max
        let mut index = *index_ptr;
        if index > u32::max_value() - MAX_EPOCH_SIZE {
            return Err(net_error::OverflowError);
        }

        let size_clamp = 
            if index + MAX_EPOCH_SIZE < max_size {
                index + MAX_EPOCH_SIZE
            }
            else {
                max_size
            };

        let header : StacksBlockHeader      = read_next(buf, &mut index, size_clamp)?;
        let txs : Vec<StacksTransaction>    = read_next(buf, &mut index, size_clamp)?;

        // there must be at least one transaction (the coinbase)
        if txs.len() == 0 {
            warn!("Invalid block: Zero-transaction block");
            return Err(net_error::DeserializeError);
        }

        // all transactions must have anchor mode either OnChainOnly or Any
        // (no OffChainOnly allowed)
        for i in 0..txs.len() {
            match txs[i].anchor_mode {
                TransactionAnchorMode::OffChainOnly => {
                    warn!("Invalid block: Found offchain-only transaction");
                    return Err(net_error::DeserializeError);
                },
                _ => {}
            };
        }

        // header and transactions must be consistent
        let txid_vecs = txs
            .iter()
            .map(|tx| tx.txid().as_bytes().to_vec())
            .collect();

        let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txid_vecs);
        let tx_merkle_root = merkle_tree.root();
        
        if tx_merkle_root != header.tx_merkle_root {
            warn!("Invalid block: Tx Merkle root mismatch");
            return Err(net_error::DeserializeError);
        }

        // there is exactly one coinbase, and it's the first transaction
        match txs[0].payload {
            TransactionPayload::Coinbase(_) => {},
            _ => {
                warn!("Invalid block: First transaction is not a coinbase");
                return Err(net_error::DeserializeError);
            }
        };

        // the coinbase must be anchored
        match txs[0].anchor_mode {
            TransactionAnchorMode::OnChainOnly => {},
            _ => {
                warn!("Invaid block: Coinbase is not on-chain-only");
                return Err(net_error::DeserializeError);
            }
        };

        // only one coinbase
        let mut coinbase_count = 0;
        for tx in txs.iter() {
            match tx.payload {
                TransactionPayload::Coinbase(_) => {
                    coinbase_count += 1;
                },
                _ => {}
            }
        }

        if coinbase_count != 1 {
            warn!("Invalid block: found {} coinbases", coinbase_count);
            return Err(net_error::DeserializeError);
        }

        *index_ptr = index;
        Ok(StacksBlock {
            header,
            txs
        })
    }
}

impl StacksBlock {
    pub fn genesis() -> StacksBlock {
        let header = StacksBlockHeader::genesis();
        StacksBlock {
            header,
            txs: vec![]
        }
    }

    pub fn from_parent(parent_header: &StacksBlockHeader, parent_microblock_header: &StacksMicroblockHeader, txs: Vec<StacksTransaction>, work_delta: &StacksWorkScore, proof: &VRFProof, state_index_root: &TrieHash, microblock_pubkey_hash: &Hash160) -> StacksBlock {
        let txids = txs.iter().map(|ref tx| tx.txid().as_bytes().to_vec()).collect();
        let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txids);
        let tx_merkle_root = merkle_tree.root();
        let header = StacksBlockHeader::from_parent(parent_header, Some(parent_microblock_header), work_delta, proof, &tx_merkle_root, state_index_root, microblock_pubkey_hash);
        StacksBlock {
            header, 
            txs
        }
    }
    
    pub fn block_hash(&self) -> BlockHeaderHash {
        self.header.block_hash()
    }
    
    pub fn index_block_hash(&self, burn_hash: &BurnchainHeaderHash) -> BlockHeaderHash {
        self.header.index_block_hash(burn_hash)
    }

    /// Validate the block, except for the transactions and state root.
    /// I.e. confirm that it is okay to begin processing this block's transactions.
    /// NOTE: the caller must call header.validate_burnchain() separately
    pub fn validate(&self, parent_header: &StacksBlockHeader, parent_microblock_stream: &Vec<StacksMicroblock>) -> bool {
        if parent_microblock_stream.len() == 0 {
            // this block should attach directly to its parent
            if !self.header.validate_parent(parent_header, None) {
                test_debug!("Invalid Stacks block {}: header does not match parent", self.block_hash().to_hex());
                return false;
            }
        }
        else {
            // this block should attach to the end of this microblock stream
            if !parent_microblock_stream[0].validate(&parent_header.block_hash(), None, &parent_header.microblock_pubkey_hash) {
                test_debug!("Invalid Stacks block {}: parent microblock stream entry 0 is invalid", self.block_hash().to_hex());
                return false;
            }

            for i in 1..parent_microblock_stream.len() {
                // parent stream must all be contiguous and valid
                if !parent_microblock_stream[i].validate(&parent_microblock_stream[i-1].block_hash(), Some(parent_microblock_stream[i-1].header.sequence), &parent_header.microblock_pubkey_hash) {
                    test_debug!("Invalid Stacks block {}: parent microblock stream entry {} is invalid", self.block_hash().to_hex(), i);
                    return false;
                }
            }
            
            if !self.header.validate_parent(parent_header, Some(&parent_microblock_stream[parent_microblock_stream.len()-1].header)) {
                test_debug!("Invalid Stacks block {}: header does not match parent or microblock parent", self.block_hash().to_hex());
                return false;
            }
        }
        
        return true;
    }

    /// Find and return the coinbase transaction.  It's always the first transaction.
    /// If there are 0 coinbase txs, or more than 1, then return None
    pub fn get_coinbase_tx(&self) -> Option<StacksTransaction> {
        if self.txs.len() == 0 {
            return None;
        }
        match self.txs[0].payload {
            TransactionPayload::Coinbase(_) => {
                Some(self.txs[0].clone())
            },
            _ => {
                None
            }
        }
    }
}


impl StacksMessageCodec for StacksMicroblockHeader {
    fn serialize(&self) -> Vec<u8> {
        let mut ret = vec![];
        write_next(&mut ret, &self.version);
        write_next(&mut ret, &self.sequence);
        write_next(&mut ret, &self.prev_block);
        write_next(&mut ret, &self.tx_merkle_root);
        write_next(&mut ret, &self.signature);
        ret
    }

    fn deserialize(buf: &Vec<u8>, index_ptr: &mut u32, max_size: u32) -> Result<StacksMicroblockHeader, net_error> {
        let mut index = *index_ptr;

        let version : u8                        = read_next(buf, &mut index, max_size)?;
        let sequence : u8                       = read_next(buf, &mut index, max_size)?;
        let prev_block : BlockHeaderHash        = read_next(buf, &mut index, max_size)?;
        let tx_merkle_root : Sha512Trunc256Sum  = read_next(buf, &mut index, max_size)?;
        let signature : MessageSignature        = read_next(buf, &mut index, max_size)?;

        // signature must be well-formed
        let _ = signature.to_secp256k1_recoverable()
            .ok_or(net_error::DeserializeError)?;
        
        *index_ptr = index;

        Ok(StacksMicroblockHeader {
            version,
            sequence,
            prev_block,
            tx_merkle_root,
            signature
        })
    }
}

impl StacksMicroblockHeader {
    pub fn sign(&mut self, privk: &StacksPrivateKey) -> Result<(), net_error> {
        self.signature = MessageSignature::empty();
        let bytes = self.serialize();
        
        let mut digest_bits = [0u8; 32];
        let mut sha2 = Sha512Trunc256::new();

        sha2.input(&bytes[..]);
        digest_bits.copy_from_slice(sha2.result().as_slice());

        let sig = privk.sign(&digest_bits)
            .map_err(|se| net_error::SigningError(se.to_string()))?;

        self.signature = sig;
        Ok(())
    }

    pub fn verify(&mut self, pubk_hash: &Hash160) -> Result<(), net_error> {
        let mut digest_bits = [0u8; 32];
        let mut sha2 = Sha512Trunc256::new();

        let sig_bits = self.signature.clone();
        self.signature = MessageSignature::empty();
        let bytes = self.serialize();
        self.signature = sig_bits;
        
        sha2.input(&bytes[..]);
        digest_bits.copy_from_slice(sha2.result().as_slice());

        let pubk = StacksPublicKey::recover_to_pubkey(&digest_bits, &self.signature)
            .map_err(|_ve| net_error::VerifyingError("Failed to verify signature: failed to recover public key".to_string()))?;
        
        if StacksBlockHeader::pubkey_hash(&pubk) != *pubk_hash {
            return Err(net_error::VerifyingError("Failed to verify signature: public key did not recover to expected hash".to_string()));
        }

        Ok(())
    }

    pub fn block_hash(&self) -> BlockHeaderHash {
        let bytes = self.serialize();
        BlockHeaderHash::from_serialized_header(&bytes[..])
    }
    
    /// Create the genesis block microblock header
    pub fn genesis() -> StacksMicroblockHeader {
        StacksMicroblockHeader {
            version: 0,
            sequence: 0,
            prev_block: BlockHeaderHash([0u8; 32]),
            tx_merkle_root: Sha512Trunc256Sum([0u8; 32]),
            signature: MessageSignature::empty()
        }
    }

    /// Create the first microblock header in a microblock stream.
    /// The header will not be signed
    pub fn first_unsigned(parent_block_hash: &BlockHeaderHash, tx_merkle_root: &Sha512Trunc256Sum) -> StacksMicroblockHeader {
        StacksMicroblockHeader {
            version: 0,
            sequence: 0,
            prev_block: parent_block_hash.clone(),
            tx_merkle_root: tx_merkle_root.clone(),
            signature: MessageSignature::empty()
        }
    }
    
    /// Create the first microblock header in a microblock stream for an empty microblock stream.
    /// The header will not be signed
    pub fn first_empty_unsigned(parent_block_hash: &BlockHeaderHash) -> StacksMicroblockHeader {
        StacksMicroblockHeader::first_unsigned(parent_block_hash, &Sha512Trunc256Sum([0u8; 32]))
    }

    /// Create an unsigned microblock header from its parent
    /// Return an error on overflow
    pub fn from_parent_unsigned(parent_header: &StacksMicroblockHeader, tx_merkle_root: &Sha512Trunc256Sum) -> Option<StacksMicroblockHeader> {
        let next_sequence = match parent_header.sequence.checked_add(1) {
            Some(next) => {
                next
            },
            None => {
                return None;
            }
        };

        Some(StacksMicroblockHeader {
            version: 0,
            sequence: next_sequence,
            prev_block: parent_header.block_hash(),
            tx_merkle_root: tx_merkle_root.clone(),
            signature: MessageSignature::empty()
        })
    }

    /// Verify that this microblock connects to its parent microblock (or anchored block), and that
    /// it was signed by the anchored block's miner.
    pub fn validate_parent(&self, parent_hash: &BlockHeaderHash, parent_sequence: Option<u8>, parent_pubkey_hash: &Hash160) -> bool {
        match parent_sequence {
            Some(seq) => {
                match seq.checked_add(1) {
                    Some(my_seq) => {
                        if self.sequence != my_seq {
                            test_debug!("Invalid microblock {}: sequence {} != {}", self.block_hash().to_hex(), self.sequence, my_seq);
                            return false;
                        }
                    },
                    None => {
                        // parent sequence is the largest sequence, so this block cannot be valid.
                        // A subsequent leader can declare this microblock as a duplicate, and
                        // steal this block's miner's coinbase.
                        test_debug!("Invalid microblock {}: sequence {} overflow", self.block_hash().to_hex(), self.sequence);
                        return false;
                    }
                };
            },
            None => {
                if self.sequence != 0 {
                    test_debug!("Invalid microblock {}: sequence {} != 0", self.block_hash().to_hex(), self.sequence);
                    return false;
                }
            }
        }

        if self.prev_block != *parent_hash {
            test_debug!("Invalid microblock {}: parent hash {} != {}", self.block_hash().to_hex(), self.prev_block.to_hex(), parent_hash.to_hex());
            return false;
        }

        let mut dup = self.clone();
        if dup.verify(parent_pubkey_hash).is_err() {
            test_debug!("Invalid microblock {}: failed to verify", self.block_hash().to_hex());
            return false;
        }

        return true;
    }
}


impl StacksMessageCodec for StacksMicroblock {
    fn serialize(&self) -> Vec<u8> {
        let mut ret = vec![];
        write_next(&mut ret, &self.header);
        write_next(&mut ret, &self.txs);
        ret
    }

    fn deserialize(buf: &Vec<u8>, index_ptr: &mut u32, max_size: u32) -> Result<StacksMicroblock, net_error> {
        // no matter what, do not allow us to parse a block bigger than the maximal epoch size
        let mut index = *index_ptr;

        if index > u32::max_value() - MAX_EPOCH_SIZE {
            return Err(net_error::OverflowError);
        }

        let size_clamp = 
            if index + MAX_EPOCH_SIZE < max_size {
                index + MAX_EPOCH_SIZE
            }
            else {
                max_size
            };

        let header : StacksMicroblockHeader = read_next(buf, &mut index, size_clamp)?;
        let txs : Vec<StacksTransaction>    = read_next(buf, &mut index, size_clamp)?;

        // all transactions must have anchor mode either OffChainOnly or Any
        // (no OnChainOnly allowed)
        for i in 0..txs.len() {
            match txs[i].anchor_mode {
                TransactionAnchorMode::OffChainOnly | TransactionAnchorMode::Any => {
                    continue;
                }
                _ => {
                    return Err(net_error::DeserializeError);
                }
            };
        }

        // header and transactions must be consistent
        let txid_vecs = txs
            .iter()
            .map(|tx| tx.txid().as_bytes().to_vec())
            .collect();

        let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txid_vecs);
        let tx_merkle_root = merkle_tree.root();
        
        if tx_merkle_root != header.tx_merkle_root {
            return Err(net_error::DeserializeError);
        }

        // there is no coinbase
        for tx in txs.iter() {
            match tx.payload {
                TransactionPayload::Coinbase(_) => {
                    return Err(net_error::DeserializeError);
                }
                _ => {}
            }
        }

        *index_ptr = index;
        Ok(StacksMicroblock {
            header,
            txs
        })
    }
}


impl StacksMicroblock {
    pub fn first_unsigned(parent_block_hash: &BlockHeaderHash, txs: Vec<StacksTransaction>) -> StacksMicroblock {
        let txids = txs.iter().map(|ref tx| tx.txid().as_bytes().to_vec()).collect();
        let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txids);
        let tx_merkle_root = merkle_tree.root();
        let header = StacksMicroblockHeader::first_unsigned(parent_block_hash, &tx_merkle_root);
        StacksMicroblock {
            header: header,
            txs: txs
        }
    }

    pub fn from_parent_unsigned(parent_header: &StacksMicroblockHeader, txs: Vec<StacksTransaction>) -> Option<StacksMicroblock> {
        let txids = txs.iter().map(|ref tx| tx.txid().as_bytes().to_vec()).collect();
        let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txids);
        let tx_merkle_root = merkle_tree.root();
        let header = match StacksMicroblockHeader::from_parent_unsigned(parent_header, &tx_merkle_root) {
            Some(h) => {
                h
            },
            None => {
                return None;
            }
        };

        Some(StacksMicroblock {
            header: header,
            txs: txs
        })
    }

    pub fn sign(&mut self, privk: &StacksPrivateKey) -> Result<(), net_error> {
        self.header.sign(privk)
    }

    pub fn verify(&mut self, pubk_hash: &Hash160) -> Result<(), net_error> {
        self.header.verify(pubk_hash)
    }

    pub fn block_hash(&self) -> BlockHeaderHash {
        self.header.block_hash()
    }

    pub fn validate(&self, parent_hash: &BlockHeaderHash, parent_sequence: Option<u8>, parent_pubkey_hash: &Hash160) -> bool {
        self.header.validate_parent(parent_hash, parent_sequence, parent_pubkey_hash)
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use chainstate::stacks::*;
    use chainstate::stacks::test::*;
    use net::*;
    use net::codec::*;
    use net::codec::test::*;

    use util::hash::*;

    #[test]
    fn codec_stacks_block_ecvrf_proof() {
        let proof_bytes = hex_bytes("024c1484fcb05cecdb4dbfb9bf4e08e7f529aea3b3a2515716ad4e9cf7bace6c91181b6bb7d8201c5a85a11c626d1848aa2ac4d188c7e24a94faa32d1eec48d600fad7c55c7e71adb6a7dd6c73f6fc02").unwrap();
        let proof = VRFProof::from_bytes(&proof_bytes[..].to_vec()).unwrap();

        check_codec_and_corruption::<VRFProof>(&proof, &proof_bytes);
    }
   
    #[test]
    fn codec_stacks_block_work_score() {
        let work_score = StacksWorkScore {
            burn: 123,
            work: 456
        };
        let work_score_bytes = vec![
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 123,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 200,
        ];
        
        check_codec_and_corruption::<StacksWorkScore>(&work_score, &work_score_bytes);
    }

    #[test]
    fn codec_stacks_block_header() {
        let proof_bytes = hex_bytes("024c1484fcb05cecdb4dbfb9bf4e08e7f529aea3b3a2515716ad4e9cf7bace6c91181b6bb7d8201c5a85a11c626d1848aa2ac4d188c7e24a94faa32d1eec48d600fad7c55c7e71adb6a7dd6c73f6fc02").unwrap();
        let proof = VRFProof::from_bytes(&proof_bytes[..].to_vec()).unwrap();

        let header = StacksBlockHeader {
            version: 0x12,
            total_work: StacksWorkScore {
                burn: 123,
                work: 456,
            },
            proof: proof,
            parent_block: BlockHeaderHash([0u8; 32]),
            parent_microblock: BlockHeaderHash([1u8; 32]),
            parent_microblock_sequence: 3,
            tx_merkle_root: Sha512Trunc256Sum([2u8; 32]),
            state_index_root: TrieHash([3u8; 32]),
            microblock_pubkey_hash: Hash160([4u8; 20])
        };

        let header_bytes = vec![
            // version
            0x12,
            // work score
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 123,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 200,
            // proof
            0x02, 0x4c, 0x14, 0x84, 0xfc, 0xb0, 0x5c, 0xec, 0xdb, 0x4d, 0xbf, 0xb9, 0xbf, 0x4e, 0x08, 0xe7, 0xf5, 0x29, 0xae, 0xa3, 
            0xb3, 0xa2, 0x51, 0x57, 0x16, 0xad, 0x4e, 0x9c, 0xf7, 0xba, 0xce, 0x6c, 0x91, 0x18, 0x1b, 0x6b, 0xb7, 0xd8, 0x20, 0x1c, 
            0x5a, 0x85, 0xa1, 0x1c, 0x62, 0x6d, 0x18, 0x48, 0xaa, 0x2a, 0xc4, 0xd1, 0x88, 0xc7, 0xe2, 0x4a, 0x94, 0xfa, 0xa3, 0x2d, 
            0x1e, 0xec, 0x48, 0xd6, 0x00, 0xfa, 0xd7, 0xc5, 0x5c, 0x7e, 0x71, 0xad, 0xb6, 0xa7, 0xdd, 0x6c, 0x73, 0xf6, 0xfc, 0x02,
            // parent block
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // parent microblock
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            // parent microblock sequence
            0x03,
            // tx merkle root
            0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
            // state index root
            0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
            // public key hash buf
            0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04
        ];

        check_codec_and_corruption::<StacksBlockHeader>(&header, &header_bytes);
    }
    
    #[test]
    fn codec_stacks_microblock_header() {
        let header = StacksMicroblockHeader {
            version: 0x12,
            sequence: 0x34,
            prev_block: BlockHeaderHash([0u8; 32]),
            tx_merkle_root: Sha512Trunc256Sum([1u8; 32]),
            signature: MessageSignature([2u8; 65]),
        };

        let header_bytes = vec![
            // version
            0x12,
            // sequence
            0x34,
            // prev block
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // tx merkle root
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            // signature
            0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
            0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
            0x02
        ];

        check_codec_and_corruption::<StacksMicroblockHeader>(&header, &header_bytes);
    }

    #[test]
    fn codec_stacks_block() {
        let proof_bytes = hex_bytes("024c1484fcb05cecdb4dbfb9bf4e08e7f529aea3b3a2515716ad4e9cf7bace6c91181b6bb7d8201c5a85a11c626d1848aa2ac4d188c7e24a94faa32d1eec48d600fad7c55c7e71adb6a7dd6c73f6fc02").unwrap();
        let proof = VRFProof::from_bytes(&proof_bytes[..].to_vec()).unwrap();

        let privk = StacksPrivateKey::from_hex("6d430bb91222408e7706c9001cfaeb91b08c2be6d5ac95779ab52c6b431950e001").unwrap();
        let origin_auth = TransactionAuth::Standard(TransactionSpendingCondition::new_singlesig_p2pkh(StacksPublicKey::from_private(&privk)).unwrap());
        let mut tx_coinbase = StacksTransaction::new(TransactionVersion::Mainnet,
                                                     origin_auth.clone(),
                                                     TransactionPayload::Coinbase(CoinbasePayload([0u8; 32])));

        tx_coinbase.anchor_mode = TransactionAnchorMode::OnChainOnly;

        // make a block with each and every kind of transaction
        let mut all_txs = codec_all_transactions(&TransactionVersion::Testnet, 0x80000000, &TransactionAnchorMode::OnChainOnly, &TransactionPostConditionMode::Allow);
        
        // remove all coinbases, except for an initial coinbase
        let mut txs_anchored = vec![];
        txs_anchored.push(tx_coinbase);

        for tx in all_txs.drain(..) {
            match tx.payload {
                TransactionPayload::Coinbase(_) => {
                    continue;
                },
                _ => {}
            }
            txs_anchored.push(tx);
        }

        let txid_vecs = txs_anchored
            .iter()
            .map(|tx| tx.txid().as_bytes().to_vec())
            .collect();

        let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txid_vecs);
        let tx_merkle_root = merkle_tree.root();
        let tr = tx_merkle_root.as_bytes().to_vec();

        let work_score = StacksWorkScore {
            burn: 123,
            work: 456
        };

        let parent_header = StacksBlockHeader {
            version: 0x01,
            total_work: StacksWorkScore {
                burn: 234,
                work: 567,
            },
            proof: proof.clone(),
            parent_block: BlockHeaderHash([5u8; 32]),
            parent_microblock: BlockHeaderHash([6u8; 32]),
            parent_microblock_sequence: 4,
            tx_merkle_root: Sha512Trunc256Sum([7u8; 32]),
            state_index_root: TrieHash([8u8; 32]),
            microblock_pubkey_hash: Hash160([9u8; 20])
        };
        
        let parent_microblock_header = StacksMicroblockHeader {
            version: 0x12,
            sequence: 0x34,
            prev_block: BlockHeaderHash([0x0au8; 32]),
            tx_merkle_root: Sha512Trunc256Sum([0x0bu8; 32]),
            signature: MessageSignature([0x0cu8; 65]),
        };

        let mut block = StacksBlock::from_parent(&parent_header, &parent_microblock_header, txs_anchored.clone(), &work_score, &proof, &TrieHash([2u8; 32]), &Hash160([3u8; 20]));
        block.header.version = 0x24;

        let ph = parent_header.block_hash().as_bytes().to_vec();
        let mh = parent_microblock_header.block_hash().as_bytes().to_vec();

        let mut block_bytes = vec![
            // header
            // version
            0x24,
            // work score (parent work score + current work score)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 123,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 200,
            // proof
            0x02, 0x4c, 0x14, 0x84, 0xfc, 0xb0, 0x5c, 0xec, 0xdb, 0x4d, 0xbf, 0xb9, 0xbf, 0x4e, 0x08, 0xe7, 0xf5, 0x29, 0xae, 0xa3, 
            0xb3, 0xa2, 0x51, 0x57, 0x16, 0xad, 0x4e, 0x9c, 0xf7, 0xba, 0xce, 0x6c, 0x91, 0x18, 0x1b, 0x6b, 0xb7, 0xd8, 0x20, 0x1c, 
            0x5a, 0x85, 0xa1, 0x1c, 0x62, 0x6d, 0x18, 0x48, 0xaa, 0x2a, 0xc4, 0xd1, 0x88, 0xc7, 0xe2, 0x4a, 0x94, 0xfa, 0xa3, 0x2d, 
            0x1e, 0xec, 0x48, 0xd6, 0x00, 0xfa, 0xd7, 0xc5, 0x5c, 0x7e, 0x71, 0xad, 0xb6, 0xa7, 0xdd, 0x6c, 0x73, 0xf6, 0xfc, 0x02,
            // parent block
            ph[0], ph[1], ph[2], ph[3], ph[4], ph[5], ph[6], ph[7], ph[8], ph[9], ph[10], ph[11], ph[12], ph[13], ph[14], ph[15], ph[16], ph[17], ph[18], ph[19], ph[20], ph[21], ph[22], ph[23], ph[24], ph[25], ph[26], ph[27], ph[28], ph[29], ph[30], ph[31],
            // parent microblock
            mh[0], mh[1], mh[2], mh[3], mh[4], mh[5], mh[6], mh[7], mh[8], mh[9], mh[10], mh[11], mh[12], mh[13], mh[14], mh[15], mh[16], mh[17], mh[18], mh[19], mh[20], mh[21], mh[22], mh[23], mh[24], mh[25], mh[26], mh[27], mh[28], mh[29], mh[30], mh[31],
            // parent microblock sequence
            0x34,
            // tx merkle root
            tr[0], tr[1], tr[2], tr[3], tr[4], tr[5], tr[6], tr[7], tr[8], tr[9], tr[10], tr[11], tr[12], tr[13], tr[14], tr[15], tr[16], tr[17], tr[18], tr[19], tr[20], tr[21], tr[22], tr[23], tr[24], tr[25], tr[26], tr[27], tr[28], tr[29], tr[30], tr[31],
            // state index root
            0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
            // public key hash buf
            0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
        ];

        check_codec_and_corruption::<StacksBlockHeader>(&block.header, &block_bytes);

        block_bytes.append(&mut txs_anchored.serialize());

        eprintln!("block is {} bytes with {} txs", block_bytes.len(), txs_anchored.len());
        check_codec_and_corruption::<StacksBlock>(&block, &block_bytes);
    }
    
    #[test]
    fn codec_stacks_microblock() { 
        // make a block with each and every kind of transaction
        let mut all_txs = codec_all_transactions(&TransactionVersion::Testnet, 0x80000000, &TransactionAnchorMode::OffChainOnly, &TransactionPostConditionMode::Allow);

        // remove all coinbases
        let mut txs_anchored = vec![];

        for tx in all_txs.drain(..) {
            match tx.payload {
                TransactionPayload::Coinbase(_) => {
                    continue;
                },
                _ => {}
            }
            txs_anchored.push(tx);
        }

        // make microblocks with 3 transactions each (or fewer)
        for i in 0..(all_txs.len() / 3) {
            let txs = vec![
                all_txs[3*i].clone(),
                all_txs[3*i+1].clone(),
                all_txs[3*i+2].clone()
            ];

            let txid_vecs = txs
                .iter()
                .map(|tx| tx.txid().as_bytes().to_vec())
                .collect();

            let merkle_tree = MerkleTree::<Sha512Trunc256Sum>::new(&txid_vecs);
            let tx_merkle_root = merkle_tree.root();
            let tr = tx_merkle_root.as_bytes().to_vec();

            let header = StacksMicroblockHeader {
                version: 0x12,
                sequence: 0x34,
                prev_block: BlockHeaderHash([0u8; 32]),
                tx_merkle_root: tx_merkle_root,
                signature: MessageSignature([0x00, 0x35, 0x44, 0x45, 0xa1, 0xdc, 0x98, 0xa1, 0xbd, 0x27, 0x98, 0x4d, 0xbe, 0x69, 0x97, 0x9a,
                                             0x5c, 0xd7, 0x78, 0x86, 0xb4, 0xd9, 0x13, 0x4a, 0xf5, 0xc4, 0x0e, 0x63, 0x4d, 0x96, 0xe1, 0xcb,
                                             0x44, 0x5b, 0x97, 0xde, 0x5b, 0x63, 0x25, 0x82, 0xd3, 0x17, 0x04, 0xf8, 0x67, 0x06, 0xa7, 0x80,
                                             0x88, 0x6e, 0x6e, 0x38, 0x1b, 0xfe, 0xd6, 0x52, 0x28, 0x26, 0x73, 0x58, 0x26, 0x2d, 0x20, 0x3f,
                                             0xe6]),
            };

            let mut block_bytes = vec![
                // header
                // version
                0x12,
                // sequence
                0x34,
                // prev block
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                // tx merkle root
                tr[0], tr[1], tr[2], tr[3], tr[4], tr[5], tr[6], tr[7], tr[8], tr[9], tr[10], tr[11], tr[12], tr[13], tr[14], tr[15], tr[16], tr[17], tr[18], tr[19], tr[20], tr[21], tr[22], tr[23], tr[24], tr[25], tr[26], tr[27], tr[28], tr[29], tr[30], tr[31],
                // signature
                0x00, 0x35, 0x44, 0x45, 0xa1, 0xdc, 0x98, 0xa1, 0xbd, 0x27, 0x98, 0x4d, 0xbe, 0x69, 0x97, 0x9a,
                0x5c, 0xd7, 0x78, 0x86, 0xb4, 0xd9, 0x13, 0x4a, 0xf5, 0xc4, 0x0e, 0x63, 0x4d, 0x96, 0xe1, 0xcb,
                0x44, 0x5b, 0x97, 0xde, 0x5b, 0x63, 0x25, 0x82, 0xd3, 0x17, 0x04, 0xf8, 0x67, 0x06, 0xa7, 0x80,
                0x88, 0x6e, 0x6e, 0x38, 0x1b, 0xfe, 0xd6, 0x52, 0x28, 0x26, 0x73, 0x58, 0x26, 0x2d, 0x20, 0x3f,
                0xe6
            ];

            block_bytes.append(&mut txs.serialize());

            let mblock = StacksMicroblock {
                header: header,
                txs: txs
            };

            check_codec_and_corruption::<StacksMicroblock>(&mblock, &block_bytes);
        }
    }

    // TODO:
    // * blocks themselves
    // * wellformed (and not wellformed) blocks
    // * size limits
}
