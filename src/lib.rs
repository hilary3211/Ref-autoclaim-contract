use near_sdk::{
    near,
    env,
    NearToken,
    Promise,
    AccountId,
    BorshStorageKey,
    PanicOnDefault,
    Timestamp,
    PromiseError,
    borsh::{ self, BorshSerialize },
    store::LazyOption,
    json_types::U128,
    Gas,
    PromiseOrValue,
    PromiseResult,
};
use serde_json::{ json, Value };
use core::fmt;

#[derive(Debug, PartialEq)]
enum ContractError {
    Unauthorized(String),
    InsufficientBalance {
        available: u128,
        requested: u128,
    },
    SerializationFailed(String),
    InvalidAccountId(String),
    AlreadyInitialized,
    InvalidInput(String),
    InsufficientDeposit(String),
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
            Self::InsufficientBalance { available, requested } =>
                write!(f, "Insufficient balance: available {}, requested {}", available, requested),
            Self::SerializationFailed(msg) => write!(f, "Serialization failed: {}", msg),
            Self::InvalidAccountId(id) => write!(f, "Invalid account ID: {}", id),
            Self::AlreadyInitialized => write!(f, "Contract has already been initialized"),
            Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            Self::InsufficientDeposit(msg) => env::panic_str(&msg),
        }
    }
}

impl ContractError {
    fn panic(&self) -> ! {
        env::panic_str(&self.to_string())
    }
}

mod config {
    use near_sdk::Gas;

    pub const MAIN_ID: &str = "auto-claim-main2.near";
    pub const BOOSTFARM: &str = "boostfarm.ref-labs.near";
    pub const REF_FINANCE: &str = "v2.ref-finance.near";
    pub const BURROW: &str = "contract.main.burrow.near";
    pub const WRAP: &str = "wrap.near";
    pub const ORACLE: &str = "priceoracle.near";

    pub const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(5);
    pub const GAS_FT_TRANSFER: Gas = Gas::from_tgas(50);
    pub const GAS_CLAIM_REWARD: Gas = Gas::from_tgas(50);
    pub const GAS_REMOVE_LIQUIDITY: Gas = Gas::from_tgas(60);
    pub const GAS_ORACLE_CALL: Gas = Gas::from_tgas(70);
    pub const GAS_NEAR_DEPOSIT: Gas = Gas::from_tgas(20);
}

const NEAR_DECIMALS: u8 = 24;
const REF_DECIMALS: u8 = 18;
const SLIPPAGE: u128 = 97;

#[derive(BorshStorageKey, BorshSerialize)]
enum StorageKey {
    Owner,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct ProxyContract {
    owner: LazyOption<AccountId>,
    last_compound_call: Timestamp,
    is_processing: bool,
}

#[near]
impl ProxyContract {
    #[init]
    pub fn new() -> Self {
        let mut owner = LazyOption::new(StorageKey::Owner, None);
        if owner.get().is_some() {
            ContractError::AlreadyInitialized.panic();
        }
        owner.set(Some(env::predecessor_account_id()));
        Self {
            owner,
            last_compound_call: 0,
            is_processing: false,
        }
    }

    fn assert_owner(&self) {
        let predecessor_id = env::predecessor_account_id();
        let stored_owner = self.owner.get().as_ref().expect("Contract not initialized");
        let main_id = config::MAIN_ID
            .parse::<AccountId>()
            .unwrap_or_else(|_|
                ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic()
            );

        if predecessor_id != *stored_owner && predecessor_id != main_id {
            ContractError::Unauthorized(
                format!(
                    "Only owner ({}) or main account ({}) can call this function",
                    stored_owner,
                    config::MAIN_ID
                )
            ).panic();
        }
    }

    fn assert_not_processing(&self) {
        if self.is_processing {
            env::panic_str("Contract is processing another transaction");
        }
    }

    fn parse_preference(pref: &Value) -> Option<(String, AccountId, AccountId, bool, String)> {
        let seed_id = pref
            .get("seed_id")
            .and_then(|v| v.as_str())
            .map(String::from)?;
        let token_id_str = pref.get("token_id").and_then(|v| v.as_str())?;
        let token_id: AccountId = token_id_str.parse().ok()?;
        let smart_contract_name_str = pref.get("smart_contract_name").and_then(|v| v.as_str())?;
        let smart_contract_name: AccountId = smart_contract_name_str.parse().ok()?;
        let is_active = pref
            .get("is_active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let reinvest_to = pref
            .get("reinvest_to")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                env::log_str("Missing or invalid reinvest_to, defaulting to 'Burrow'");
                "Burrow".to_string()
            });

        Some((seed_id, token_id, smart_contract_name, is_active, reinvest_to))
    }

    //This function is used for the old smart contract that uses true as string instead of bool
    // fn parse_preference(pref: &Value) -> Option<(String, AccountId, AccountId, bool, String)> {
    //     let seed_id = pref.get("seed_id").and_then(|v| v.as_str()).map(String::from)?;
    //     if seed_id.len() > 64 {
    //         env::log_str("seed_id exceeds maximum length of 64 characters");
    //         return None;
    //     }
    //     let token_id = pref.get("token_id")
    //         .and_then(|v| v.as_str())
    //         .and_then(|s| s.parse().ok())?;
    //     let smart_contract_name = pref.get("smart_contract_name")
    //         .and_then(|v| v.as_str())
    //         .and_then(|s| s.parse().ok())?;

    //     let is_active = pref.get("is_active")
    //         .and_then(|v| v.as_str())
    //         .map(|s| s.to_lowercase() == "true")
    //         .unwrap_or(false);
    //     let reinvest_to = pref.get("reinvest_to")
    //         .and_then(|v| v.as_str())
    //         .map(String::from)
    //         .unwrap_or_else(|| {
    //             env::log_str("Missing or invalid reinvest_to, defaulting to 'Burrow'");
    //             "Burrow".to_string()
    //         });

    //     Some((seed_id, token_id, smart_contract_name, is_active, reinvest_to))
    // }

    pub fn get_owner(&self) -> AccountId {
        self.owner.get().as_ref().expect("Contract not initialized").clone()
    }

    #[private]
    pub fn stake_x_ref(
        &mut self,
        smart_contract_name: AccountId,
        deposit_amount: U128,
        min_amount_out: U128,
        wnear_to_ref_pool_id: String
    ) -> Promise {
        if deposit_amount.0 == 0 {
            ContractError::InvalidInput("deposit_amount must be non-zero".to_string()).panic();
        }
        if wnear_to_ref_pool_id.is_empty() {
            ContractError::InvalidInput("wnear_to_ref_pool_id cannot be empty".to_string()).panic();
        }

        let p_0 = Promise::new(config::WRAP.parse().unwrap()).function_call(
            "near_deposit".to_string(),
            "{}".to_string().into_bytes(),
            NearToken::from_yoctonear(deposit_amount.0),
            config::GAS_STORAGE_DEPOSIT
        );

        let p_1 = Promise::new(config::WRAP.parse().unwrap()).function_call(
            "ft_transfer_call".to_string(),
            json!({
                    "receiver_id": "v2.ref-finance.near",
                    "amount": deposit_amount,
                    "msg": json!({
                        "actions": [{
                            "pool_id": wnear_to_ref_pool_id,
                            "token_in": config::WRAP,
                            "token_out": "token.v2.ref-finance.near",
                            "amount_in": deposit_amount,
                            "min_amount_out": min_amount_out,
                        }]
                    }).to_string()
                })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(25)
        );

        p_0.then(p_1).then(
            Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(100))
                .stake_x_ref_callback(
                    smart_contract_name,
                    deposit_amount,
                    min_amount_out,
                    wnear_to_ref_pool_id
                )
        )
    }

    #[private]
    pub fn stake_x_ref_callback(
        &mut self,
        smart_contract_name: AccountId,
        deposit_amount: U128,
        min_amount_out: U128,
        wnear_to_ref_pool_id: String,
        #[callback_result] swap_result: Result<U128, PromiseError>
    ) -> Promise {
        match swap_result {
            Ok(ref_amount) => {
                env::log_str(&format!("Swap succeeded, received {} REF", ref_amount.0));
                if ref_amount.0 < min_amount_out.0 {
                    env::panic_str(
                        &format!(
                            "Received REF amount {} below min_amount_out {}",
                            ref_amount.0,
                            min_amount_out.0
                        )
                    );
                }

                env::log_str("Proceeding to REF-to-xREF conversion");
                Promise::new("token.v2.ref-finance.near".parse().unwrap())
                    .function_call(
                        "ft_transfer_call".to_string(),
                        json!({"receiver_id": "xtoken.ref-finance.near", "amount": ref_amount, "msg": ""})
                            .to_string()
                            .into_bytes(),
                        NearToken::from_yoctonear(1),
                        Gas::from_tgas(25)
                    )
                    .then(
                        Self::ext(env::current_account_id())
                            .with_static_gas(Gas::from_tgas(100))
                            .stake_x_ref_transfer_callback(
                                smart_contract_name,
                                ref_amount,
                                min_amount_out
                            )
                    )
            }
            Err(e) => {
                let error_str = format!("{:?}", e);
                env::log_str(&format!("Swap error: {}", error_str));
                let wrap_storage_cost = NearToken::from_millinear(125);
                let ref_storage_cost = NearToken::from_millinear(125);
                let total_required = wrap_storage_cost.saturating_add(ref_storage_cost);
                env::log_str(
                    &format!(
                        "Storage check: wrap_cost={} yoctoNEAR, ref_cost={} yoctoNEAR, total_required={} yoctoNEAR",
                        wrap_storage_cost.as_yoctonear(),
                        ref_storage_cost.as_yoctonear(),
                        total_required.as_yoctonear()
                    )
                );

                let p_1 = Promise::new(config::WRAP.parse().unwrap()).function_call(
                    "storage_deposit".to_string(),
                    json!({"account_id": smart_contract_name.to_string(), "registration_only": true})
                        .to_string()
                        .into_bytes(),
                    wrap_storage_cost,
                    config::GAS_STORAGE_DEPOSIT
                );

                let p_2 = Promise::new("token.v2.ref-finance.near".parse().unwrap()).function_call(
                    "storage_deposit".to_string(),
                    json!({"account_id": smart_contract_name.to_string(), "registration_only": true})
                        .to_string()
                        .into_bytes(),
                    ref_storage_cost,
                    config::GAS_STORAGE_DEPOSIT
                );

                let p_3 = Promise::new(config::WRAP.parse().unwrap()).function_call(
                    "ft_transfer_call".to_string(),
                    json!({
                                "receiver_id": "v2.ref-finance.near",
                                "amount": deposit_amount,
                                "msg": json!({
                                    "actions": [{
                                        "pool_id": wnear_to_ref_pool_id,
                                        "token_in": config::WRAP,
                                        "token_out": "token.v2.ref-finance.near",
                                        "amount_in": deposit_amount,
                                        "min_amount_out": min_amount_out,
                                    }]
                                }).to_string()
                            })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(1),
                    Gas::from_tgas(25)
                );

                p_1.then(p_2)
                    .then(p_3)
                    .then(
                        Self::ext(env::current_account_id())
                            .with_static_gas(Gas::from_tgas(50))
                            .stake_x_ref_callback(
                                smart_contract_name,
                                deposit_amount,
                                min_amount_out,
                                wnear_to_ref_pool_id
                            )
                    )
            }
        }
    }

    #[private]
    pub fn stake_x_ref_transfer_callback(
        &mut self,
        smart_contract_name: AccountId,
        ref_amount: U128,
        min_amount_out: U128,
        #[callback_result] transfer_result: Result<(), PromiseError>
    ) -> Promise {
        match transfer_result {
            Ok(_) => {
                env::log_str(
                    &format!("Transfer to xtoken.ref-finance.near succeeded, {} REF", ref_amount.0)
                );
                Promise::new(env::current_account_id())
            }
            Err(e) => {
                let error_str = format!("{:?}", e);
                if
                    error_str.contains("Storage") ||
                    error_str.contains("NotEnoughBalance") ||
                    error_str.contains("registered")
                {
                    let ref_storage_cost = NearToken::from_millinear(125);
                    let xtoken_storage_cost = NearToken::from_millinear(125);
                    let total_required = ref_storage_cost.saturating_add(xtoken_storage_cost);
                    env::log_str(
                        &format!(
                            "Storage check: ref_cost={} yoctoNEAR, xtoken_cost={} yoctoNEAR, total_required={} yoctoNEAR",
                            ref_storage_cost.as_yoctonear(),
                            xtoken_storage_cost.as_yoctonear(),
                            total_required.as_yoctonear()
                        )
                    );

                    let p_1 = Promise::new(
                        "token.v2.ref-finance.near".parse().unwrap()
                    ).function_call(
                        "storage_deposit".to_string(),
                        json!({"account_id": smart_contract_name.to_string(), "registration_only": true})
                            .to_string()
                            .into_bytes(),
                        ref_storage_cost,
                        config::GAS_STORAGE_DEPOSIT
                    );

                    let p_2 = Promise::new(
                        "xtoken.ref-finance.near".parse().unwrap()
                    ).function_call(
                        "storage_deposit".to_string(),
                        json!({"account_id": smart_contract_name.to_string(), "registration_only": true})
                            .to_string()
                            .into_bytes(),
                        xtoken_storage_cost,
                        config::GAS_STORAGE_DEPOSIT
                    );

                    let retry_transfer = Promise::new(
                        "token.v2.ref-finance.near".parse().unwrap()
                    ).function_call(
                        "ft_transfer_call".to_string(),
                        json!({"receiver_id": "xtoken.ref-finance.near", "amount": ref_amount, "msg": ""})
                            .to_string()
                            .into_bytes(),
                        NearToken::from_yoctonear(1),
                        Gas::from_tgas(25)
                    );

                    p_1.then(p_2).then(retry_transfer)
                } else if error_str.contains("invalid msg") || error_str.contains("expected value") {
                    env::panic_str(
                        &format!(
                            "Invalid msg format for xREF transfer: amount={}, min_amount_out={}. Error: {:?}",
                            ref_amount.0,
                            min_amount_out.0,
                            e
                        )
                    );
                } else {
                    env::panic_str(
                        &format!(
                            "Transfer to xtoken.ref-finance.near failed: amount={}, min_amount_out={}. Error: {:?}",
                            ref_amount.0,
                            min_amount_out.0,
                            e
                        )
                    );
                }
            }
        }
    }

    #[private]
    pub fn on_pool_data_fetched(
        &self,
        #[callback_result] pool_result: Result<Value, PromiseError>
    ) -> U128 {
        let pool_data = match pool_result {
            Ok(value) => value,
            Err(e) => {
                env::log_str(&format!("Failed to fetch pool data: {:?}", e));
                env::panic_str("Pool data fetch failed");
            }
        };

        env::log_str(&format!("Raw pool data: {}", pool_data.to_string()));

        let token_account_ids = pool_data
            .get("token_account_ids")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                ContractError::InvalidInput(
                    format!("Invalid token_account_ids: {}", pool_data.to_string())
                ).panic()
            });

        if token_account_ids.len() != 2 {
            ContractError::InvalidInput(
                format!("Pool must have exactly two tokens, got: {}", pool_data.to_string())
            ).panic();
        }

        let token0 = token_account_ids
            .get(0)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                ContractError::InvalidInput(
                    format!("Missing token0: {}", pool_data.to_string())
                ).panic()
            });
        let token1 = token_account_ids
            .get(1)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                ContractError::InvalidInput(
                    format!("Missing token1: {}", pool_data.to_string())
                ).panic()
            });

        if token0 != "token.v2.ref-finance.near" || token1 != config::WRAP {
            ContractError::InvalidInput(
                format!("Pool must be REF/wNEAR, got: {} and {}", token0, token1)
            ).panic();
        }

        let amounts = pool_data
            .get("amounts")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                ContractError::InvalidInput(
                    format!("Invalid amounts: {}", pool_data.to_string())
                ).panic()
            });

        let ref_amount = amounts
            .get(0)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or_else(|| {
                ContractError::InvalidInput(
                    format!("Invalid ref_amount: {}", pool_data.to_string())
                ).panic()
            });

        let near_amount = amounts
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or_else(|| {
                ContractError::InvalidInput(
                    format!("Invalid near_amount: {}", pool_data.to_string())
                ).panic()
            });

        if near_amount == 0 {
            ContractError::InvalidInput("Pool has no NEAR liquidity".to_string()).panic();
        }
        if ref_amount == 0 {
            ContractError::InvalidInput("Pool has no REF liquidity".to_string()).panic();
        }

        let decimal_adjustment = (10u128).pow((NEAR_DECIMALS - REF_DECIMALS) as u32);
        let base_rate = (ref_amount * decimal_adjustment) / near_amount;
        let effective_rate = (base_rate * SLIPPAGE) / 100;

        env::log_str(&format!("NEAR to REF rate (with 3% slippage): {} (scaled)", effective_rate));
        U128(effective_rate)
    }

    pub fn compound(&mut self, pool_id: u64) -> Promise {
        // self.assert_not_processing();
        // self.is_processing = false;

        let current_time = env::block_timestamp();
        assert!(
            current_time - self.last_compound_call >= 3_600_000_000_000,
            "Compound can only be called once every hour"
        );

        let caller = env::predecessor_account_id();
        let owner = self.owner.get().as_ref().expect("Contract not initialized").clone();
        let main_account_id: AccountId = config::MAIN_ID
            .parse()
            .unwrap_or_else(|_|
                ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic()
            );

        let get_pool_promise = Promise::new(config::REF_FINANCE.parse().unwrap())
            .function_call(
                "get_pool".to_string(),
                json!({"pool_id": pool_id}).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(2)
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "on_pool_data_fetched".to_string(),
                    json!({})
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(2)
                )
            );

        let get_user_promise = Promise::new(main_account_id).function_call(
            "get_user".to_string(),
            json!({"wallet_id": owner}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(2)
        );

        get_pool_promise
            .and(get_user_promise)
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "compound_callback".to_string(),
                    json!({"pre_claim_balance": U128(env::account_balance().as_yoctonear()), "caller": caller,  "pool_id": pool_id})
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(280)
                )
            )
    }

    #[private]
    pub fn compound_callback(
        &mut self,
        pre_claim_balance: U128,
        caller: AccountId,
        pool_id: u64
    ) -> PromiseOrValue<Vec<Promise>> {
        assert_eq!(
            env::predecessor_account_id(),
            env::current_account_id(),
            "Callback can only be called by the contract itself"
        );

        env::log_str(&format!("Promise results count: {}", env::promise_results_count()));
        if env::promise_results_count() != 2 {
            self.is_processing = false;
            env::log_str("Unexpected number of promise results");
            return PromiseOrValue::Value(vec![]);
        }

        let mut all_promises = Vec::new();

        let rate = match env::promise_result(0) {
            PromiseResult::Successful(result) => {
                env::log_str(&format!("Raw pool result: {:?}", result));
                serde_json::from_slice::<U128>(&result).unwrap_or_else(|e| {
                    env::log_str(&format!("Serialization error: {}", e));
                    ContractError::SerializationFailed(e.to_string()).panic()
                })
            }
            PromiseResult::Failed => {
                self.is_processing = false;
                env::log_str("Failed to fetch NEAR-to-REF rate");
                return PromiseOrValue::Value(vec![]);
            }
        };

        match env::promise_result(1) {
            PromiseResult::Successful(result) => {
                let user_data: Value = serde_json
                    ::from_slice(&result)
                    .unwrap_or_else(|e| ContractError::SerializationFailed(e.to_string()).panic());

                let preferences = match user_data.get("preferences").and_then(|v| v.as_array()) {
                    Some(prefs) => prefs,
                    None => {
                        env::log_str("No preferences found in user data");
                        &Vec::new()
                    }
                };

                let mut claim_promises = Vec::new();

                for pref in preferences {
                    let (seed_id, token_id, smart_contract_name, is_active, reinvest_to) = match
                        Self::parse_preference(pref)
                    {
                        Some(data) => data,
                        None => {
                            continue;
                        }
                    };

                    if !is_active {
                        continue;
                    }

                    if reinvest_to == "Burrow" {
                        let claim_all_rewards = Promise::new(
                            env::current_account_id()
                        ).function_call(
                            "claim_all_rewards".to_string(),
                            json!({"seed_id": seed_id, "token_id": token_id})
                                .to_string()
                                .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(20)
                        );

                        let claim_from_burrow = Promise::new(
                            env::current_account_id()
                        ).function_call(
                            "claim_from_burrow".to_string(),
                            json!({})
                                .to_string()
                                .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(50)
                        );

                        let handle_promise = claim_all_rewards.and(claim_from_burrow);

                        claim_promises.push(handle_promise);
                    } else {
                        let claim_all_rewards = Promise::new(
                            env::current_account_id()
                        ).function_call(
                            "claim_all_rewards".to_string(),
                            json!({"seed_id": seed_id, "token_id": token_id})
                                .to_string()
                                .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(20)
                        );

                        let handle_promise = claim_all_rewards;

                        claim_promises.push(handle_promise);
                    }
                }

                if claim_promises.is_empty() {
                    self.last_compound_call = env::block_timestamp();
                    env::log_str("No active preferences to process");
                    return PromiseOrValue::Value(vec![]);
                }

                let combined_promise = claim_promises
                    .into_iter()
                    .reduce(|acc, promise| acc.and(promise))
                    .unwrap()
                    .then(
                        Promise::new(env::current_account_id()).function_call(
                            "finalize_compound".to_string(),
                            json!({
                                    "pre_claim_balance": pre_claim_balance,
                                    "caller": caller,
                                    "near_to_ref_rate": rate,
                                    "pool_id": pool_id,
                                    "reinvest_to": preferences.get(0).and_then(|p| p.get("reinvest_to")).and_then(|v| v.as_str()).unwrap_or("Burrow"),
                                    "smart_contract_name": preferences.get(0).and_then(|p| p.get("smart_contract_name")).and_then(|v| v.as_str()).unwrap_or("")
                                })
                                .to_string()
                                .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(230)
                        )
                    );

                all_promises.push(combined_promise);
            }
            PromiseResult::Failed => {
                self.is_processing = false;
                env::log_str("Failed to retrieve user data");
            }
        }

        PromiseOrValue::Value(all_promises)
    }

    #[private]
    pub fn finalize_compound(
        &mut self,
        pre_claim_balance: U128,
        caller: AccountId,
        near_to_ref_rate: U128,
        pool_id: u64,
        reinvest_to: String,
        smart_contract_name: String
    ) -> PromiseOrValue<Vec<Promise>> {
        let predecessor_id = env::predecessor_account_id();
        let current_id = env::current_account_id();
        let owner_id = self.owner.get().as_ref().expect("Contract not initialized");

        if predecessor_id != current_id && predecessor_id != *owner_id {
            env::panic_str("Callback can only be called by the contract itself or the owner");
        }

        self.last_compound_call = env::block_timestamp();

        let mut final_promises = Vec::new();
        let post_claim_balance = env::account_balance().as_yoctonear();
        let balance_increase = post_claim_balance.saturating_sub(pre_claim_balance.0);

        env::log_str(
            &format!(
                "Pre-claim: {}, Post-claim: {}, Increase: {}",
                pre_claim_balance.0,
                post_claim_balance,
                balance_increase
            )
        );

        let threshold = 2_000_000_000_000_000_000_000_000;
        if balance_increase < threshold {
            let caller_share_near = (balance_increase * 5) / 100;

            let remaining_balance = (balance_increase * 95) / 100;
            let caller_share_ref =
                (remaining_balance * near_to_ref_rate.0) / (10u128).pow(NEAR_DECIMALS as u32);

            env::log_str(
                &format!(
                    "Reinvesting {} yoctoREF for caller {} (from {} yoctoNEAR at rate {})",
                    caller_share_ref,
                    &caller,
                    caller_share_near,
                    near_to_ref_rate.0
                )
            );

            final_promises.push(
                Promise::new(env::current_account_id()).function_call(
                    "reinvest".to_string(),
                    json!({
                            "contract_id": smart_contract_name.parse::<AccountId>().unwrap_or_else(|_| ContractError::InvalidAccountId(smart_contract_name.clone()).panic()),
                            "min_amount_out": U128(caller_share_ref),
                            "deposit_amount" : U128(remaining_balance),
                            "pool_id": pool_id.to_string(),
                            "reinvest_to": reinvest_to,
                        })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(200)
                )
            );
            final_promises.push(
                Promise::new(caller).transfer(NearToken::from_yoctonear(caller_share_near))
            );
        } else {
            env::log_str(
                &format!(
                    "Claimed amount {} yoctoNEAR below threshold of {} yoctoNEAR",
                    balance_increase,
                    threshold
                )
            );
        }

        self.is_processing = false;
        PromiseOrValue::Value(final_promises)
    }

    #[private]
    pub fn reinvest(
        &mut self,
        contract_id: AccountId,
        min_amount_out: U128,
        deposit_amount: U128,
        pool_id: String,
        reinvest_to: String
    ) -> Vec<Promise> {
        let mut promises = Vec::new();

        let balance = env::account_balance().as_yoctonear();
        let two_near = 2_000_000_000_000_000_000_000_000;

        if balance <= two_near {
            env::log_str(&format!("Insufficient balance for reinvestment: {}", balance));
            return promises;
        }

        if reinvest_to == "Burrow" {
            let deposit_promise = Promise::new(contract_id.clone())
                .function_call(
                    "deposit_into_burrow".to_string(),
                    json!({
                        "deposit_amount": deposit_amount,
                    })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(50)
                )
                .then(
                    Promise::new(env::current_account_id()).function_call(
                        "handle_reinvest_result".to_string(),
                        json!({
                                "contract_name": contract_id.to_string(),
                                "action": "deposit_into_burrow"
                            })
                            .to_string()
                            .into_bytes(),
                        NearToken::from_yoctonear(0),
                        Gas::from_tgas(3)
                    )
                );
            promises.push(deposit_promise);
        } else if reinvest_to == "Stake" {
            let swap_promise = Promise::new(contract_id.clone())
                .function_call(
                    "stake_x_ref".to_string(),
                    json!({
                        "smart_contract_name": contract_id.to_string(),
                        "deposit_amount": deposit_amount,
                        "min_amount_out": min_amount_out,
                        "wnear_to_ref_pool_id": pool_id,
                    })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(190)
                )
                .then(
                    Promise::new(env::current_account_id()).function_call(
                        "handle_reinvest_result".to_string(),
                        json!({
                                "contract_name": contract_id.to_string(),
                                "action": "stake_x_ref"
                            })
                            .to_string()
                            .into_bytes(),
                        NearToken::from_yoctonear(0),
                        Gas::from_tgas(3)
                    )
                );
            promises.push(swap_promise);
        } else {
            ContractError::InvalidInput(
                format!("Invalid reinvest_to option: {}", reinvest_to)
            ).panic();
        }

        promises
    }

    #[private]
    pub fn handle_reinvest_result(
        &self,
        contract_name: String,
        action: String
    ) -> PromiseOrValue<()> {
        if env::promise_results_count() > 0 {
            match env::promise_result(0) {
                PromiseResult::Successful(_) => {
                    env::log_str(&format!("{} succeeded for {}", action, contract_name));
                }
                PromiseResult::Failed => {
                    env::log_str(
                        &format!(
                            "Failed to execute {} on {}: Check account registration or balance",
                            action,
                            contract_name
                        )
                    );
                }
            }
        } else {
            env::log_str("No promise result available");
        }
        PromiseOrValue::Value(())
    }

    pub fn stake_lp_tokens(
        &mut self,
        pool_id: String,
        lp_token_amount: U128,
        user_account: AccountId
    ) -> Promise {
        self.assert_owner();

        if pool_id.is_empty() {
            ContractError::InvalidInput("pool_id cannot be empty".to_string()).panic();
        }
        if lp_token_amount.0 == 0 {
            ContractError::InvalidInput("lp_token_amount must be non-zero".to_string()).panic();
        }

        let min_deposit = NearToken::from_yoctonear(1);
        let attached = env::attached_deposit();
        if attached < min_deposit {
            ContractError::InsufficientDeposit(
                format!(
                    "Attached deposit {} NEAR is less than required {} NEAR",
                    attached.as_near(),
                    min_deposit.as_near()
                )
            ).panic();
        }

        let transfer_promise = Promise::new(config::REF_FINANCE.parse().unwrap())
            .function_call(
                "mft_transfer_call".to_string(),
                json!({
                "receiver_id": config::BOOSTFARM,
                "token_id": pool_id,
                "amount": lp_token_amount,
                "msg": "\"Free\""
            })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(1),
                config::GAS_FT_TRANSFER
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(100))
                    .stake_lp_tokens_callback(pool_id, lp_token_amount, user_account)
            );

        transfer_promise
    }

    #[private]
    pub fn stake_lp_tokens_callback(
        &mut self,
        pool_id: String,
        lp_token_amount: U128,
        user_account: AccountId,
        #[callback_result] transfer_result: Result<Value, PromiseError>
    ) -> Promise {
        match transfer_result {
            Ok(value) => {
                env::log_str(&format!("Transfer succeeded with result: {:?}", value));
                Promise::new(env::current_account_id())
            }
            Err(e) => {
                let error_str = format!("{:?}", e);
                let ref_storage_cost = NearToken::from_millinear(125);
                let boost_storage_cost = NearToken::from_millinear(100);

                let storage_deposit_promise = Promise::new(
                    config::REF_FINANCE.parse().unwrap()
                ).function_call(
                    "storage_deposit".to_string(),
                    json!({"account_id": user_account.to_string(), "registration_only": false})
                        .to_string()
                        .into_bytes(),
                    ref_storage_cost,
                    config::GAS_STORAGE_DEPOSIT
                );
                let storage_deposit_promise2 = Promise::new(
                    config::BOOSTFARM.parse().unwrap()
                ).function_call(
                    "storage_deposit".to_string(),
                    json!({"account_id": user_account.to_string(), "registration_only": false})
                        .to_string()
                        .into_bytes(),
                    boost_storage_cost,
                    config::GAS_STORAGE_DEPOSIT
                );
                let retry_transfer = Promise::new(
                    config::REF_FINANCE.parse().unwrap()
                ).function_call(
                    "mft_transfer_call".to_string(),
                    json!({
                            "receiver_id": config::BOOSTFARM,
                            "token_id": pool_id,
                            "amount": lp_token_amount,
                            "msg": ""
                        })
                        .to_string()
                        .into_bytes(),
                    NearToken::from_yoctonear(1),
                    config::GAS_FT_TRANSFER
                );
                storage_deposit_promise.then(storage_deposit_promise2).then(retry_transfer)
            }
        }
    }

    pub fn unstake_lp(
        &mut self,
        seed_id: String,
        withdraw_amount: U128,
        token_id: AccountId
    ) -> Promise {
        self.assert_owner();
        if seed_id.is_empty() {
            ContractError::InvalidInput("seed_id cannot be empty".to_string()).panic();
        }
        if withdraw_amount.0 == 0 {
            ContractError::InvalidInput("withdraw_amount must be non-zero".to_string()).panic();
        }

        let unlock_and_withdraw_seed = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
                )
        ).function_call(
            "unlock_and_withdraw_seed".to_string(),
            json!({
                "seed_id": seed_id,
                "unlock_amount": "0",
                "withdraw_amount": withdraw_amount,
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        );

        let withdraw_reward_token = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
                )
        ).function_call(
            "withdraw_reward".to_string(),
            json!({"token_id": token_id}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD
        );

        unlock_and_withdraw_seed.then(withdraw_reward_token)
    }

    #[private]
    pub fn claim_all_rewards(&mut self, seed_id: String, token_id: AccountId) -> Promise {
        if seed_id.is_empty() {
            ContractError::InvalidInput("seed_id cannot be empty".to_string()).panic();
        }

        let claim_reward = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
                )
        ).function_call(
            "claim_reward_by_seed".to_string(),
            json!({"seed_id": seed_id, "token_id": token_id}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(6)
        );

        let withdraw_lp_reward = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
                )
        ).function_call(
            "withdraw_reward".to_string(),
            json!({"token_id": token_id}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(6)
        );

        claim_reward.then(withdraw_lp_reward)
    }

    pub fn remove_liquidity_and_withdraw_tokens(
        &mut self,
        pool_id: u64,
        shares: U128,
        min_token_amount: U128,
        min_wrapped_near_amount: U128,
        token_id: AccountId,
        second_token_id: AccountId
    ) -> Promise {
        self.assert_owner();
        if shares.0 == 0 {
            ContractError::InvalidInput("shares must be non-zero".to_string()).panic();
        }

        let remove_liquidity = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic()
                )
        ).function_call(
            "remove_liquidity".to_string(),
            json!({
                "pool_id": pool_id,
                "shares": shares,
                "min_amounts": [min_token_amount, min_wrapped_near_amount],
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_REMOVE_LIQUIDITY
        );

        let withdraw_wrap = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic()
                )
        ).function_call(
            "withdraw".to_string(),
            json!({
                "token_id": token_id,
                "amount": "0",
                "unregister": false,
                "skip_unwrap_near": false,
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        );

        let withdraw_black = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic()
                )
        ).function_call(
            "withdraw".to_string(),
            json!({
                "token_id": second_token_id,
                "amount": "0",
                "unregister": false,
                "skip_unwrap_near": false,
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        );

        remove_liquidity.then(withdraw_wrap).then(withdraw_black)
    }

    pub fn deposit_into_burrow(&mut self, deposit_amount: U128) -> Promise {
        if deposit_amount.0 == 0 {
            ContractError::InvalidInput("deposit_amount must be non-zero".to_string()).panic();
        }

        let storage_deposit_promise = Promise::new(
            config::BURROW
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::BURROW.to_string()).panic()
                )
        ).function_call(
            "storage_deposit".to_string(),
            json!({})
                .to_string()
                .into_bytes(),
            NearToken::from_millinear(250),
            config::GAS_STORAGE_DEPOSIT
        );

        let deposit_promise = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
                )
        ).function_call(
            "near_deposit".to_string(),
            json!({})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(deposit_amount.0),
            config::GAS_NEAR_DEPOSIT
        );

        let collateral_transfer = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
                )
        ).function_call(
            "ft_transfer_call".to_string(),
            json!({
                "receiver_id": config::BURROW,
                "amount": deposit_amount,
                "msg": json!({"Execute": {"actions": [{"IncreaseCollateral": {"token_id": config::WRAP, "max_amount": deposit_amount}}]}}).to_string()
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        );

        storage_deposit_promise.then(deposit_promise).then(collateral_transfer)
    }

    pub fn deposit_into_burrow_pool(
        &mut self,
        token_id: AccountId,
        deposit_amount: U128
    ) -> Promise {
        if deposit_amount.0 == 0 {
            ContractError::InvalidInput("deposit_amount must be non-zero".to_string()).panic();
        }

        Promise::new(token_id.clone()).function_call(
            "ft_transfer_call".to_string(),
            json!({
                "receiver_id": config::BURROW,
                "amount": deposit_amount,
                "msg": json!({"Execute": {"actions": [{"IncreaseCollateral": {"token_id": token_id, "max_amount": deposit_amount}}]}}).to_string()
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        )
    }

    pub fn claim_from_burrow(&mut self) -> Promise {
        Promise::new(
            config::BURROW
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::BURROW.to_string()).panic()
                )
        ).function_call(
            "account_farm_claim_all".to_string(),
            json!({})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD
        )
    }

    pub fn withdraw_from_borrow_pool(&mut self, withdraw_amount: U128) -> Promise {
        self.assert_owner();
        if withdraw_amount.0 == 0 {
            ContractError::InvalidInput("withdraw_amount must be non-zero".to_string()).panic();
        }

        let wrap_promise = Promise::new(
            config::ORACLE
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::ORACLE.to_string()).panic()
                )
        ).function_call(
            "oracle_call".to_string(),
            json!({
                "receiver_id": config::BURROW,
                "msg": json!({"Execute": {"actions": [
                    {"DecreaseCollateral": {"token_id": config::WRAP}},
                    {"Withdraw": {"token_id": config::WRAP}}
                ]}}).to_string()
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_ORACLE_CALL
        );

        let wrap_promise2 = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_|
                    ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
                )
        ).function_call(
            "near_withdraw".to_string(),
            json!({"amount": withdraw_amount}).to_string().into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        );

        wrap_promise.then(wrap_promise2)
    }

    pub fn withdraw_token(
        &mut self,
        token_id: AccountId,
        receiver_id: AccountId,
        amount: U128
    ) -> Promise {
        self.assert_owner();
        let attached_deposit = env::attached_deposit();
        if attached_deposit != NearToken::from_yoctonear(1) {
            env::panic_str("Requires exactly 1 yoctoNEAR attached deposit");
        }

        if amount.0 == 0 {
            ContractError::InvalidInput("amount must be non-zero".to_string()).panic();
        }

        Promise::new(token_id.clone()).function_call(
            "ft_transfer".to_string(),
            json!({
                "receiver_id": receiver_id,
                "amount": amount,
                "memo": "Withdraw token from contract"
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER
        )
    }

    pub fn get_contract_balance(&self) -> NearToken {
        self.assert_owner();
        env::account_balance()
    }

    #[payable]
    pub fn withdraw_amount(&mut self, beneficiary: AccountId, amount: U128) -> Promise {
        self.assert_owner();

        let attached_deposit = env::attached_deposit();
        if attached_deposit != NearToken::from_yoctonear(1) {
            env::panic_str("Requires exactly 1 yoctoNEAR attached deposit");
        }

        if amount.0 == 0 {
            ContractError::InvalidInput("amount must be non-zero".to_string()).panic();
        }

        let available_balance = env::account_balance();
        let storage_cost = (env::storage_usage() as u128) * env::storage_byte_cost().as_yoctonear();
        let total_balance = available_balance.as_yoctonear() + storage_cost;
        let amount_near = NearToken::from_yoctonear(amount.0);

        if available_balance < amount_near {
            env::log_str(
                &format!(
                    "Total balance: {}, Storage reserved: {}, Available: {}",
                    total_balance,
                    storage_cost,
                    available_balance.as_yoctonear()
                )
            );
            (ContractError::InsufficientBalance {
                available: available_balance.as_yoctonear(),
                requested: amount_near.as_yoctonear(),
            }).panic();
        }

        env::log_str(
            &format!(
                "Initiating transfer of {} yoctoNEAR to beneficiary: {}",
                amount.0,
                beneficiary
            )
        );
        Promise::new(beneficiary).transfer(amount_near)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{ VMContextBuilder, accounts };
    use near_sdk::testing_env;

    #[test]
    fn test_new() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let contract = ProxyContract::new();
        assert_eq!(contract.get_owner(), accounts(0));
    }

    #[test]
    fn test_assert_owner_success() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let contract = ProxyContract::new();
        contract.assert_owner();
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_assert_owner_failure() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let contract = ProxyContract::new();
        let context = VMContextBuilder::new().predecessor_account_id(accounts(1)).build();
        testing_env!(context);
        contract.assert_owner();
    }

    #[test]
    #[should_panic(expected = "Invalid input: pool_id cannot be empty")]
    fn test_stake_lp_tokens_empty_pool_id() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_lp_tokens("".to_string(), U128(1_000), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
    fn test_stake_x_ref_zero_deposit() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_x_ref(accounts(1), U128(0), U128(0), "123".to_string());
    }

    #[test]
    #[should_panic(expected = "Invalid input: wnear_to_ref_pool_id cannot be empty")]
    fn test_stake_x_ref_empty_pool_id() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_x_ref(accounts(1), U128(1_000), U128(0), "".to_string());
    }

    #[test]
    fn test_reinvest_insufficient_balance() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .account_balance(NearToken::from_near(1))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        let promises = contract.reinvest(
            accounts(1),
            U128(0),
            "123".to_string(),
            "Burrow".to_string()
        );
        assert_eq!(promises.len(), 0);
    }

    #[test]
    fn test_withdraw_amount_with_storage() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(1))
            .account_balance(NearToken::from_near(5))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        let _promise = contract.withdraw_amount(
            accounts(1),
            U128(1_000_000_000_000_000_000_000_000)
        );
    }

    #[test]
    #[should_panic(expected = "Compound can only be called once every hour")]
    fn test_compound_multiple_callers() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(1))
            .block_timestamp(0)
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();

        let promises = contract.compound(U128(1000));
        assert_eq!(promises.len(), 1);

        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(2))
            .block_timestamp(1_000_000_000)
            .build();
        testing_env!(context);
    }
}
