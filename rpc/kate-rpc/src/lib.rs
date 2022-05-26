use std::sync::{Arc, RwLock};

use codec::Encode;
use da_primitives::asdr::{AppExtrinsic, AppId, GetAppId};
use dusk_plonk::commitment_scheme::kzg10::PublicParameters;
use frame_support::storage::storage_prefix;
use frame_system::limits::BlockLength;
use jsonrpc_core::{Error as RpcError, Result};
use jsonrpc_derive::rpc;
use kate::BlockDimensions;
use kate_rpc_runtime_api::KateParamsGetter;
use lru::LruCache;
use sc_client_api::{BlockBackend, StorageKey, StorageProvider};
use sp_api::{HashT, ProvideRuntimeApi};
use sp_blockchain::HeaderBackend;
use sp_rpc::number::NumberOrHex;
use sp_runtime::{
	generic::BlockId,
	traits::{BlakeTwo256, Block as BlockT, Header, NumberFor},
};

pub type VRFSeed = [u8; 32];

#[rpc]
pub trait KateApi {
	#[rpc(name = "kate_queryProof")]
	fn query_proof(
		&self,
		block_number: NumberOrHex,
		cells: Vec<kate::com::Cell>,
	) -> Result<Vec<u8>>;

	#[rpc(name = "kate_blockLength")]
	fn query_block_length(&self) -> Result<BlockLength>;
}

macro_rules! internal_err {
	($($arg:tt)*) => {{
		let mut error = RpcError::internal_error();
		error.message = format!($($arg)*);
		error
	}}
}

pub struct Kate<Client, Block: BlockT> {
	client: Arc<Client>,
	block_ext_cache:
		RwLock<LruCache<Block::Hash, (Vec<dusk_plonk::prelude::BlsScalar>, BlockDimensions)>>,
}

impl<Client, Block> Kate<Client, Block>
where
	Block: BlockT,
{
	pub fn new(client: Arc<Client>) -> Self {
		Self {
			client,
			block_ext_cache: RwLock::new(LruCache::new(2048)), // 524288 bytes per block, ~1Gb max size
		}
	}
}

impl<Client, Block> Kate<Client, Block>
where
	Block: BlockT,
	Client: ProvideRuntimeApi<Block> + StorageProvider<Block, sc_client_db::Backend<Block>>,
	Client::Api: KateParamsGetter<Block>,
{
	pub fn random_seed(&self, block_id: &BlockId<Block>) -> VRFSeed { self.vrf(block_id) }

	/// Fetches the VRF or `[0u8;32]` if VRF is not available yet.
	fn vrf(&self, block_id: &BlockId<Block>) -> VRFSeed {
		Self::runtime_vrf(self.client.clone(), block_id)
			.or_else(|_| Self::raw_vrf(self.client.clone(), block_id))
			.unwrap_or_default()
	}

	/// Fetches the VRF using the Runtime.
	fn runtime_vrf(client: Arc<Client>, block_id: &BlockId<Block>) -> Result<VRFSeed> {
		client
			.runtime_api()
			.get_babe_vrf(&block_id)
			.map_err(|e| internal_err!("Runtime Babe VRF not found at block {}: {:?}", block_id, e))
	}

	/// Fetches the VRF using raw access on legacy Runtimes.
	fn raw_vrf(client: Arc<Client>, block_id: &BlockId<Block>) -> Result<VRFSeed> {
		let randomness_key = StorageKey(storage_prefix(b"Babe", b"Randomness").to_vec());
		let epoc_and_block = client
			.storage(block_id, &randomness_key)
			.map_err(|_| internal_err!("Babe::Randomness key is invalid"))?
			.ok_or_else(|| internal_err!("Missing Babe::Randomness at block {:?}", block_id))?
			.0;

		let seed = BlakeTwo256::hash_of(&epoc_and_block);
		log::info!("VRF Seed {:?} for block {:?}", seed, block_id);

		Ok(seed.into())
	}
}

/// Error type of this RPC api.
pub enum Error {
	/// The transaction was not decodable.
	DecodeError,
	/// The call to runtime failed.
	RuntimeError,
}

impl From<Error> for i64 {
	fn from(e: Error) -> i64 {
		match e {
			Error::RuntimeError => 1,
			Error::DecodeError => 2,
		}
	}
}

impl<Client, Block> KateApi for Kate<Client, Block>
where
	Block: BlockT,
	Block::Extrinsic: GetAppId<AppId>,
	Client: Send + Sync + 'static,
	Client: HeaderBackend<Block>
		+ ProvideRuntimeApi<Block>
		+ BlockBackend<Block>
		+ StorageProvider<Block, sc_client_db::Backend<Block>>,
	Client::Api: KateParamsGetter<Block>,
{
	//TODO allocate static thread pool, just for RPC related work, to free up resources, for the block producing processes.
	fn query_proof(
		&self,
		block_number: NumberOrHex,
		cells: Vec<kate::com::Cell>,
	) -> Result<Vec<u8>> {
		let block_num: u32 = block_number
			.try_into()
			.map_err(|_| RpcError::invalid_params("Invalid block number"))?;

		let block_num = <NumberFor<Block>>::from(block_num);
		let signed_block = self
			.client
			.block(&BlockId::number(block_num))
			.map_err(|e| internal_err!("Invalid block number: {:?}", e))?
			.ok_or_else(|| internal_err!("Missing block number {}", block_num))?;
		let block_hash = signed_block.block.header().hash();
		let block_id = BlockId::hash(block_hash.clone());

		let mut block_ext_cache = self
			.block_ext_cache
			.write()
			.map_err(|_| internal_err!("Block cache lock is poisoned .qed"))?;

		let block_length: BlockLength = self
			.client
			.runtime_api()
			.get_block_length(&block_id)
			.map_err(|e| internal_err!("Block Length cannot be fetched: {:?}", e))?;

		if !block_ext_cache.contains(&block_hash) {
			// build block data extension and cache it
			let xts_by_id: Vec<AppExtrinsic> = signed_block
				.block
				.extrinsics()
				.iter()
				.map(|e| AppExtrinsic {
					app_id: e.app_id(),
					data: e.encode(),
				})
				.collect();

			// Use Babe's VRF
			let seed = self.random_seed(&block_id);
			log::info!("VRF Seed {:?} for block {:?}", seed, block_id);

			let (_, block, block_dims) = kate::com::flatten_and_pad_block(
				block_length.rows as usize,
				block_length.cols as usize,
				block_length.chunk_size() as usize,
				&xts_by_id,
				seed,
			)
			.map_err(|e| internal_err!("Flatten and pad block failed: {:?}", e))?;

			let data = kate::com::extend_data_matrix(block_dims, &block)
				.map_err(|e| internal_err!("Matrix cannot be extended: {:?}", e))?;
			block_ext_cache.put(block_hash, (data, block_dims));
		}

		let (ext_data, block_dims) = block_ext_cache
			.get(&block_hash)
			.ok_or_else(|| internal_err!("Block hash {} cannot be fetched", block_hash))?;
		let kc_public_params_raw = self
			.client
			.runtime_api()
			.get_public_params(&block_id)
			.map_err(|e| {
				internal_err!(
					"Public params cannot be fetched on block {}: {:?}",
					block_hash,
					e
				)
			})?;
		let kc_public_params =
			unsafe { PublicParameters::from_slice_unchecked(&kc_public_params_raw) };

		let proof = kate::com::build_proof(&kc_public_params, *block_dims, ext_data, &cells)
			.map_err(|e| internal_err!("Proof cannot be generated: {:?}", e))?;

		Ok(proof)
	}

	fn query_block_length(&self) -> Result<BlockLength> {
		let best_hash = self.client.info().best_hash;
		let block_length = self
			.client
			.runtime_api()
			.get_block_length(&BlockId::hash(best_hash))
			.map_err(|e| internal_err!("Length of best block({:?}): {:?}", best_hash, e))?;

		Ok(block_length)
	}
}
