//! Test utilies for the Ethereum bridge crate.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::str::FromStr;

use borsh::BorshSerialize;
use namada_core::ledger::eth_bridge::storage::bridge_pool::get_key_from_hash;
use namada_core::ledger::storage::testing::{TestStorage, TestWlStorage};
use namada_core::ledger::storage_api::{StorageRead, StorageWrite};
use namada_core::types::address::{self, wnam, Address};
use namada_core::types::ethereum_events::EthAddress;
use namada_core::types::keccak::KeccakHash;
use namada_core::types::key::{self, protocol_pk_key, RefTo};
use namada_core::types::storage::{BlockHeight, Key};
use namada_core::types::token;
use namada_proof_of_stake::parameters::PosParams;
use namada_proof_of_stake::types::GenesisValidator;
use rust_decimal_macros::dec;

use crate::parameters::{
    ContractVersion, Contracts, EthereumBridgeConfig, MinimumConfirmations,
    UpgradeableContract,
};

/// Validator keys used for testing purposes.
pub struct TestValidatorKeys {
    /// Consensus keypair.
    pub consensus: key::common::SecretKey,
    /// Protocol keypair.
    pub protocol: key::common::SecretKey,
    /// Ethereum hot keypair.
    pub eth_bridge: key::common::SecretKey,
    /// Ethereum cold keypair.
    pub eth_gov: key::common::SecretKey,
}

impl TestValidatorKeys {
    /// Generate a new test wallet.
    #[inline]
    pub fn generate() -> Self {
        TestValidatorKeys {
            consensus: key::common::SecretKey::Ed25519(
                key::testing::gen_keypair::<key::ed25519::SigScheme>(),
            ),
            protocol: key::common::SecretKey::Ed25519(
                key::testing::gen_keypair::<key::ed25519::SigScheme>(),
            ),
            eth_bridge: key::common::SecretKey::Secp256k1(
                key::testing::gen_keypair::<key::secp256k1::SigScheme>(),
            ),
            eth_gov: key::common::SecretKey::Secp256k1(
                key::testing::gen_keypair::<key::secp256k1::SigScheme>(),
            ),
        }
    }
}

/// Set up a [`TestWlStorage`] initialized at genesis with a single
/// validator.
///
/// The validator's address is [`address::testing::established_address_1`].
#[inline]
pub fn setup_default_storage()
-> (TestWlStorage, HashMap<Address, TestValidatorKeys>) {
    let mut wl_storage = TestWlStorage::default();
    let all_keys = init_default_storage(&mut wl_storage);
    (wl_storage, all_keys)
}

/// Set up a [`TestWlStorage`] initialized at genesis with a single
/// validator.
///
/// The validator's address is [`address::testing::established_address_1`].
#[inline]
pub fn init_default_storage(
    wl_storage: &mut TestWlStorage,
) -> HashMap<Address, TestValidatorKeys> {
    init_storage_with_validators(
        wl_storage,
        HashMap::from_iter([(
            address::testing::established_address_1(),
            100_u64.into(),
        )]),
    )
}

/// Writes a dummy [`EthereumBridgeConfig`] to the given [`TestWlStorage`], and
/// returns it.
pub fn bootstrap_ethereum_bridge(
    wl_storage: &mut TestWlStorage,
) -> EthereumBridgeConfig {
    let config = EthereumBridgeConfig {
        min_confirmations: MinimumConfirmations::from(unsafe {
            // SAFETY: The only way the API contract of `NonZeroU64` can
            // be violated is if we construct values
            // of this type using 0 as argument.
            NonZeroU64::new_unchecked(10)
        }),
        contracts: Contracts {
            native_erc20: wnam(),
            bridge: UpgradeableContract {
                address: EthAddress([2; 20]),
                version: ContractVersion::default(),
            },
            governance: UpgradeableContract {
                address: EthAddress([3; 20]),
                version: ContractVersion::default(),
            },
        },
    };
    config.init_storage(wl_storage);
    config
}

/// Returns the number of keys in `storage` which have values present.
pub fn stored_keys_count(wl_storage: &TestWlStorage) -> usize {
    let root = Key::from_str("").expect("Test failed");
    wl_storage.iter_prefix(&root).expect("Test failed").count()
}

/// Set up a [`TestWlStorage`] initialized at genesis with the given
/// validators.
pub fn setup_storage_with_validators(
    active_validators: HashMap<Address, token::Amount>,
) -> (TestWlStorage, HashMap<Address, TestValidatorKeys>) {
    let mut wl_storage = TestWlStorage::default();
    let all_keys =
        init_storage_with_validators(&mut wl_storage, active_validators);
    (wl_storage, all_keys)
}

/// Set up a [`TestWlStorage`] initialized at genesis with the given
/// validators.
pub fn init_storage_with_validators(
    wl_storage: &mut TestWlStorage,
    active_validators: HashMap<Address, token::Amount>,
) -> HashMap<Address, TestValidatorKeys> {
    // set last height to a reasonable value;
    // it should allow vote extensions to be cast
    wl_storage.storage.last_height = 3.into();

    let mut all_keys = HashMap::new();
    let validators = active_validators.into_iter().map(|(address, tokens)| {
        let keys = TestValidatorKeys::generate();
        let consensus_key = keys.consensus.ref_to();
        let eth_cold_key = keys.eth_gov.ref_to();
        let eth_hot_key = keys.eth_bridge.ref_to();
        all_keys.insert(address.clone(), keys);
        GenesisValidator {
            address,
            tokens,
            consensus_key,
            eth_cold_key,
            eth_hot_key,
            commission_rate: dec!(0.05),
            max_commission_rate_change: dec!(0.01),
        }
    });

    namada_proof_of_stake::init_genesis(
        wl_storage,
        &PosParams::default(),
        validators,
        0.into(),
    )
    .expect("Test failed");
    let config = EthereumBridgeConfig {
        min_confirmations: Default::default(),
        contracts: Contracts {
            native_erc20: wnam(),
            bridge: UpgradeableContract {
                address: EthAddress([42; 20]),
                version: Default::default(),
            },
            governance: UpgradeableContract {
                address: EthAddress([18; 20]),
                version: Default::default(),
            },
        },
    };
    config.init_storage(wl_storage);

    for (validator, keys) in all_keys.iter() {
        let protocol_key = keys.protocol.ref_to();
        wl_storage
            .write(&protocol_pk_key(validator), protocol_key)
            .expect("Test failed");
    }
    wl_storage.commit_block().expect("Test failed");

    all_keys
}

/// Commit a bridge pool root at a given height
/// to storage.
///
/// N.B. assumes the bridge pool is empty.
pub fn commit_bridge_pool_root_at_height(
    storage: &mut TestStorage,
    root: &KeccakHash,
    height: BlockHeight,
) {
    let value = height.try_to_vec().expect("Encoding failed");
    storage
        .block
        .tree
        .update(&get_key_from_hash(root), value)
        .unwrap();
    storage.block.height = height;
    storage.commit_block().unwrap();
    storage.block.tree.delete(&get_key_from_hash(root)).unwrap();
}
