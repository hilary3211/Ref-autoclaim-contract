use near_sdk::{
    near, env, NearToken, Promise, AccountId, BorshStorageKey, PanicOnDefault, Timestamp,
    PromiseError, borsh::{self, BorshSerialize, BorshDeserialize}, store::LazyOption, json_types::U128,
    Gas, PromiseOrValue, PromiseResult
};
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use core::fmt;

#[derive(Debug, PartialEq)]
enum ContractError {
    Unauthorized(String),
    InsufficientBalance { available: u128, requested: u128 },
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

    pub const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(6);
    pub const GAS_FT_TRANSFER: Gas = Gas::from_tgas(50);
    pub const GAS_CLAIM_REWARD: Gas = Gas::from_tgas(10);
    pub const GAS_REMOVE_LIQUIDITY: Gas = Gas::from_tgas(60);
    pub const GAS_ORACLE_CALL: Gas = Gas::from_tgas(70);
    pub const GAS_NEAR_DEPOSIT: Gas = Gas::from_tgas(10);
}

const NEAR_DECIMALS: u8 = 24;
const REF_DECIMALS: u8 = 18;
const SLIPPAGE: u128 = 97;

#[derive(BorshStorageKey, BorshSerialize)]
enum StorageKey {
    Owner,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ReinvestFrom {
    BurrowFarm { token: AccountId },
    RefBoostfarm { pool_id: u64 },
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ReinvestTo {
    BurrowMainVault { token_id: AccountId },
    RefStake,
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
            .unwrap_or_else(|_| ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic());

        if predecessor_id != *stored_owner && predecessor_id != main_id {
            ContractError::Unauthorized(format!(
                "Only owner ({}) or main account ({}) can call this function",
                stored_owner, config::MAIN_ID
            ))
            .panic();
        }
    }

    fn assert_not_processing(&self) {
        if self.is_processing {
            env::panic_str("Contract is processing another transaction");
        }
    }

    fn parse_preference(
        pref: &Value
    ) -> Option<(String, AccountId, AccountId, bool, ReinvestFrom, ReinvestTo)> {
        let seed_id = pref.get("seed_id").and_then(|v| v.as_str()).map(String::from)?;
        let token_id_str = pref.get("token_id").and_then(|v| v.as_str())?;
        let token_id: AccountId = token_id_str.parse().ok()?;
        let smart_contract_name_str = pref.get("smart_contract_name").and_then(|v| v.as_str())?;
        let smart_contract_name: AccountId = smart_contract_name_str.parse().ok()?;
        let is_active = pref.get("is_active").and_then(|v| v.as_bool()).unwrap_or(false);
        // let is_active = pref.get("is_active")
        //     .map(|v| {
        //         let result = v.as_bool()
        //             .unwrap_or_else(|| {
        //                 v.as_str()
        //                     .map(|s| s.to_lowercase() == "true")
        //                     .unwrap_or(false)
        //             });
        //         env::log_str(&format!("Parsed is_active: {}", result));
        //         result
        //     })
        //     .unwrap_or_else(|| {
        //         env::log_str("is_active missing, defaulting to false");
        //         false
        //     });

        let invested_in = match pref.get("invested_in").and_then(|v| v.as_str()) {
            Some("Burrow") => ReinvestFrom::BurrowFarm { token: token_id.clone() },
            Some("RefBoostfarm") => {
                let pool_id = seed_id.parse::<u64>().ok()?;
                ReinvestFrom::RefBoostfarm { pool_id }
            }
            _ => return None,
        };

        let reinvest_to = match pref.get("reinvest_to").and_then(|v| v.as_str()) {
            Some("Burrow") => ReinvestTo::BurrowMainVault { token_id: token_id.clone() },
            Some("Stake") => ReinvestTo::RefStake,
            _ => return None,
        };

        Some((seed_id, token_id, smart_contract_name, is_active, invested_in, reinvest_to))
    }

    pub fn get_owner(&self) -> AccountId {
        self.owner.get().as_ref().expect("Contract not initialized").clone()
    }

    pub fn compound(&mut self) -> Promise {
        let current_time = env::block_timestamp();
        assert!(
            current_time - self.last_compound_call >= 3_600_000_000_000,
            "Compound can only be called once every hour"
        );
        self.assert_not_processing();
        self.is_processing = true;

        let caller = env::predecessor_account_id();
        let owner = self.owner.get().as_ref().expect("Contract not initialized").clone();
        let main_account_id: AccountId = config::MAIN_ID
            .parse()
            .unwrap_or_else(|_| ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic());

        let get_user_promise = Promise::new(main_account_id).function_call(
            "get_user".to_string(),
            json!({"wallet_id": owner}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(2)
        );

        get_user_promise.then(
            Promise::new(env::current_account_id()).function_call(
                "compound_callback".to_string(),
                json!({"pre_claim_balance": U128(env::account_balance().as_yoctonear()), "caller": caller})
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(285)
            )
        )
    }

    #[private]
    pub fn compound_callback(
        &mut self,
        pre_claim_balance: U128,
        caller: AccountId,
    ) -> PromiseOrValue<Vec<Promise>> {
        assert_eq!(
            env::predecessor_account_id(),
            env::current_account_id(),
            "Callback can only be called by the contract itself"
        );

        env::log_str(&format!("Promise results count: {}", env::promise_results_count()));
        if env::promise_results_count() != 1 {
            self.is_processing = false;
            env::log_str("Unexpected number of promise results");
            return PromiseOrValue::Value(vec![]);
        }

        let mut all_promises = Vec::new();

        match env::promise_result(0) {
            PromiseResult::Successful(result) => {
          

                let raw_data_str = String::from_utf8_lossy(&result).to_string();
                env::log_str(&format!("Raw user data: {}", raw_data_str));

                let user_data: Value = serde_json::from_slice(&result).unwrap_or_else(|e| {
                    self.is_processing = false;
                    ContractError::SerializationFailed(e.to_string()).panic()
                });

                let preferences = match user_data.get("preferences").and_then(|v| v.as_array()) {
                    Some(prefs) => {
                        env::log_str(&format!("Found {} preferences", prefs.len()));
                        prefs
                    }
                    None => {
                        self.is_processing = false;
                        self.last_compound_call = env::block_timestamp();
                        env::log_str("No preferences array in user data");
                        return PromiseOrValue::Value(vec![]);
                    }
                };

   
                let first_active_pref = preferences.iter().enumerate().find_map(|(i, pref)| {
                    env::log_str(&format!("Processing preference #{}", i));
                    let parsed = Self::parse_preference(pref);
                    if let Some((seed_id, token_id, smart_contract_name, is_active, invested_in, reinvest_to)) = parsed {
                        if is_active {
                            env::log_str(&format!("Found active preference #{}", i));
                            Some((seed_id, token_id, smart_contract_name, invested_in, reinvest_to))
                        } else {
                            env::log_str(&format!("Preference #{} is inactive", i));
                            None
                        }
                    } else {
                        env::log_str(&format!("Preference #{} failed to parse", i));
                        None
                    }
                });


                let claim_promise = match first_active_pref {
                    Some((seed_id, _token_id, smart_contract_name, invested_in, reinvest_to)) => {
                        let promise = match invested_in {
                            ReinvestFrom::BurrowFarm { .. } => self.claim_from_burrow(),
                            ReinvestFrom::RefBoostfarm { .. } => {
                                self.claim_all_rewards(seed_id, smart_contract_name.clone())
                            }
                        };
                        Some((promise, reinvest_to, smart_contract_name))
                    }
                    None => {
                        self.is_processing = false;
                        self.last_compound_call = env::block_timestamp();
                        env::log_str("No active preferences to process");
                        None
                    }
                };

                if let Some((promise, reinvest_to, smart_contract_name)) = claim_promise {
                    let combined_promise = promise.then(
                        Promise::new(env::current_account_id()).function_call(
                            "finalize_compound".to_string(),
                            json!({
                                "pre_claim_balance": pre_claim_balance,
                                "caller": caller,
                                "reinvest_to": reinvest_to,
                                "smart_contract_name": smart_contract_name.to_string()
                            })
                            .to_string()
                            .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(180)
                        )
                    );
                    all_promises.push(combined_promise);
                }
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
        reinvest_to: String,
        smart_contract_name: String
    ) -> PromiseOrValue<Vec<Promise>> {
        let predecessor_id = env::predecessor_account_id();
        let current_id = env::current_account_id();
        let owner_id = self.owner.get().as_ref().expect("Contract not initialized");

        if predecessor_id != current_id && predecessor_id != *owner_id {
            self.is_processing = false;
            env::panic_str("Callback can only be called by the contract itself or the owner");
        }

        let reinvest_to: ReinvestTo = serde_json::from_str(&reinvest_to).unwrap_or_else(|e| {
            self.is_processing = false;
            ContractError::SerializationFailed(format!("Failed to deserialize reinvest_to: {}", e)).panic()
        });

        let mut final_promises = Vec::new();
        let post_claim_balance = env::account_balance().as_yoctonear();
        let balance_increase = post_claim_balance.saturating_sub(pre_claim_balance.0);

        env::log_str(&format!(
            "Pre-claim: {}, Post-claim: {}, Increase: {}",
            pre_claim_balance.0, post_claim_balance, balance_increase
        ));

        let threshold = 2_000_000_000_000_000_000_000_000;
        if balance_increase < threshold {
            let caller_share_near = (balance_increase * 5) / 100;
            let remaining_balance = (balance_increase * 95) / 100;

            env::log_str(&format!(
                "Reinvesting {} yoctoNEAR for caller {} (from {} yoctoNEAR)",
                remaining_balance, caller, caller_share_near
            ));

            let contract_id = match reinvest_to {
                ReinvestTo::BurrowMainVault { .. } => {
                    config::BURROW.parse::<AccountId>().unwrap()
                }
                ReinvestTo::RefStake => {
                    smart_contract_name.parse::<AccountId>().unwrap_or_else(|_| {
                        self.is_processing = false;
                        ContractError::InvalidAccountId(smart_contract_name.clone()).panic()
                    })
                }
            };

            final_promises.push(
                Promise::new(env::current_account_id()).function_call(
                    "reinvest".to_string(),
                    json!({
                        "contract_id": contract_id,
                        "deposit_amount": U128(remaining_balance),
                        "reinvest_to": reinvest_to
                    })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(140)
                )
            );
            final_promises.push(
                Promise::new(caller).transfer(NearToken::from_yoctonear(caller_share_near))
            );
        } else {
            env::log_str(&format!(
                "Claimed amount {} yoctoNEAR below threshold of {} yoctoNEAR",
                balance_increase, threshold
            ));
        }

        self.is_processing = false;
        self.last_compound_call = env::block_timestamp();
        PromiseOrValue::Value(final_promises)

       
    }

    #[private]
    pub fn reinvest(
        &mut self,
        contract_id: AccountId,
        deposit_amount: U128,
        reinvest_to: String
    ) -> Vec<Promise> {
        let mut promises = Vec::new();

        let reinvest_to: ReinvestTo = serde_json::from_str(&reinvest_to).unwrap_or_else(|e| {
            self.is_processing = false;
            ContractError::SerializationFailed(format!("Failed to deserialize reinvest_to: {}", e)).panic()
        });

        let balance = env::account_balance().as_yoctonear();
        let two_near = 2_000_000_000_000_000_000_000_000;

        if balance <= two_near {
            self.is_processing = false;
            env::log_str(&format!("Insufficient balance for reinvestment: {}", balance));
            return promises;
        }

        match reinvest_to {
            ReinvestTo::BurrowMainVault { .. } => {
                let deposit_promise = self.deposit_into_burrow(deposit_amount).then(
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
            }
            ReinvestTo::RefStake => {
                self.is_processing = false;
                ContractError::InvalidInput("RefStake not implemented".to_string()).panic();
                
            }
        }

        promises
    }

    #[private]
    pub fn claim_from_burrow(&mut self) -> Promise {
        Promise::new(config::BURROW.parse().unwrap()).function_call(
            "account_farm_claim_all".to_string(),
            json!({}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD
        )
    }

    #[private]
    pub fn claim_all_rewards(&mut self, seed_id: String, token_id: AccountId) -> Promise {
        if seed_id.is_empty() {
            ContractError::InvalidInput("seed_id cannot be empty".to_string()).panic();
        }

        Promise::new(config::BOOSTFARM.parse().unwrap()).function_call(
            "claim_reward_by_seed".to_string(),
            json!({"seed_id": seed_id, "token_id": token_id}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(6)
        )
    }

    #[payable]
    pub fn deposit_into_burrow(&mut self, deposit_amount: U128) -> Promise {
        if deposit_amount.0 == 0 {
            ContractError::InvalidInput("Deposit amount must be non-zero".to_string()).panic();
        }

        let min_deposit = NearToken::from_yoctonear(deposit_amount.0);
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

        let burrow_account: AccountId = config::BURROW
            .parse()
            .unwrap_or_else(|_| {
                ContractError::InvalidAccountId(config::BURROW.to_string()).panic()
            });
        let wrap_account: AccountId = config::WRAP
            .parse()
            .unwrap_or_else(|_| {
                ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
            });

        let deposit_promise = Promise::new(wrap_account.clone()).function_call(
            "near_deposit".to_string(),
            json!({}).to_string().into_bytes(),
            NearToken::from_yoctonear(deposit_amount.0),
            config::GAS_NEAR_DEPOSIT
        );

        let collateral_transfer = Promise::new(wrap_account.clone()).function_call(
            "ft_transfer_call".to_string(),
            json!({
                "receiver_id": config::BURROW,
                "amount": deposit_amount,
                "msg": json!({"Execute": {"actions": [{"IncreaseCollateral": {"token_id": config::WRAP, "max_amount": deposit_amount}}]}}).to_string()
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(50)
        );

        deposit_promise.then(collateral_transfer)
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

    #[payable]
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
            );

        transfer_promise
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
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic())
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
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic())
        ).function_call(
            "withdraw_reward".to_string(),
            json!({"token_id": token_id}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD
        );

        let delete_preference = Promise::new(
            config::MAIN_ID
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic())
        ).function_call(
            "delete_preference".to_string(),
            json!({"seed_id": seed_id}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(20)
        );

        unlock_and_withdraw_seed.then(withdraw_reward_token).then(delete_preference)
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

    pub fn withdraw_from_borrow_pool(&mut self, withdraw_amount: U128) -> Promise {
        self.assert_owner();
        if withdraw_amount.0 == 0 {
            ContractError::InvalidInput("withdraw_amount must be non-zero".to_string()).panic();
        }

        let wrap_promise = Promise::new(
            config::ORACLE
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::ORACLE.to_string()).panic())
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
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::WRAP.to_string()).panic())
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
            ContractError::InsufficientBalance {
                available: available_balance.as_yoctonear(),
                requested: amount_near.as_yoctonear(),
            }.panic();
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

    pub fn transfer_lp_token(
        &mut self,
        receiver_id: AccountId,
        token_id: String,
        amount: U128
    ) -> Promise {
        self.assert_owner();

        if token_id.is_empty() {
            ContractError::InvalidInput("Token ID cannot be empty".to_string()).panic();
        }
        if amount.0 == 0 {
            ContractError::InvalidInput("Amount must be non-zero".to_string()).panic();
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

        let account_id = env::current_account_id();

        Promise::new(receiver_id).function_call(
            "mft_transfer".to_string(),
            json!({
                "receiver_id": account_id.to_string(),
                "token_id": token_id,
                "amount": amount,
                "memo": null
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(85)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{VMContextBuilder, accounts};
    use near_sdk::testing_env;

    #[test]
    fn test_new() {
        let context = VMContextBuilder::new().predecessor_account_id( [accounts(0)].build());
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
    fn test_assert_owner_main_id_success() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(config::MAIN_ID.parse().unwrap())
            .build();
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
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(1))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_lp_tokens("".to_string(), U128(1_000), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Invalid input: lp_token_amount must be non-zero")]
    fn test_stake_lp_tokens_zero_amount() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(1))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_lp_tokens("pool1".to_string(), U128(0), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
    fn test_deposit_into_burrow_zero_amount() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.deposit_into_burrow(U128(0));
    }

    #[test]
    #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
    fn test_deposit_into_burrow_pool_zero_amount() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.deposit_into_burrow_pool(accounts(1), U128(0));
    }

    #[test]
    #[should_panic(expected = "Invalid input: withdraw_amount must be non-zero")]
    fn test_withdraw_from_borrow_pool_zero_amount() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.withdraw_from_borrow_pool(U128(0));
    }

    #[test]
    #[should_panic(expected = "Invalid input: seed_id cannot be empty")]
    fn test_unstake_lp_empty_seed_id() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.unstake_lp("".to_string(), U128(1_000), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Invalid input: withdraw_amount must be non-zero")]
    fn test_unstake_lp_zero_amount() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.unstake_lp("seed1".to_string(), U128(0), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Invalid input: seed_id cannot be empty")]
    fn test_claim_all_rewards_empty_seed_id() {
        let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.claim_all_rewards("".to_string(), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Requires exactly 1 yoctoNEAR attached deposit")]
    fn test_withdraw_token_invalid_deposit() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(0))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.withdraw_token(accounts(1), accounts(2), U128(1_000));
    }

    #[test]
    #[should_panic(expected = "Invalid input: amount must be non-zero")]
    fn test_withdraw_token_zero_amount() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(1))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.withdraw_token(accounts(1), accounts(2), U128(0));
    }

    #[test]
    fn test_get_contract_balance() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .account_balance(NearToken::from_near(5))
            .build();
        testing_env!(context);
        let contract = ProxyContract::new();
        let balance = contract.get_contract_balance();
        assert_eq!(balance, NearToken::from_near(5));
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_get_contract_balance_unauthorized() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .account_balance(NearToken::from_near(5))
            .build();
        testing_env!(context);
        let contract = ProxyContract::new();
        let context = VMContextBuilder::new().predecessor_account_id(accounts(1)).build();
        testing_env!(context);
        contract.get_contract_balance();
    }

    #[test]
    #[should_panic(expected = "Requires exactly 1 yoctoNEAR attached deposit")]
    fn test_withdraw_amount_invalid_deposit() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(0))
            .account_balance(NearToken::from_near(5))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.withdraw_amount(accounts(1), U128(1_000_000_000_000_000_000_000_000));
    }

    #[test]
    #[should_panic(expected = "Insufficient balance")]
    fn test_withdraw_amount_insufficient_balance() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_yoctonear(1))
            .account_balance(NearToken::from_near(1))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.withdraw_amount(accounts(1), U128(2_000_000_000_000_000_000_000_000));
    }

    #[test]
    fn test_withdraw_amount_success() {
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
    fn test_compound_too_soon() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .block_timestamp(0)
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.compound();

        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .block_timestamp(1_000_000_000_000)
            .build();
        testing_env!(context);
        contract.compound();
    }

    #[test]
    fn test_compound_callback_single_preference() {
        let mut context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context.clone());
        let mut contract = ProxyContract::new();

        let user_data = json!({
            "preferences": [
                {
                    "seed_id": "123",
                    "token_id": accounts(1).to_string(),
                    "smart_contract_name": accounts(2).to_string(),
                    "is_active": true,
                    "invested_in": "Burrow",
                    "reinvest_to": "Burrow"
                }
            ]
        });
        let user_result = serde_json::to_vec(&user_data).unwrap();

        testing_env!(
            context,
            near_sdk::VMConfig::test(),
            near_sdk::RuntimeFeesConfig::test(),
            Default::default(),
            vec![PromiseResult::Successful(user_result)]
        );

        let result = contract.compound_callback(U128(100), accounts(0));
        match result {
            PromiseOrValue::Value(promises) => assert_eq!(promises.len(), 1, "Expected one promise"),
            _ => panic!("Expected PromiseOrValue::Value"),
        }
    }
}








// use near_sdk::{
//     near,
//     env,
//     NearToken,
//     Promise,
//     AccountId,
//     BorshStorageKey,
//     PanicOnDefault,
//     Timestamp,
//     PromiseError,
//     borsh::{ self, BorshSerialize },
//     store::LazyOption,
//     json_types::U128,
//     Gas,
//     PromiseOrValue,
//     PromiseResult,
// };
// use serde_json::{ json, Value };
// use core::fmt;

// #[derive(Debug, PartialEq)]
// enum ContractError {
//     Unauthorized(String),
//     InsufficientBalance {
//         available: u128,
//         requested: u128,
//     },
//     SerializationFailed(String),
//     InvalidAccountId(String),
//     AlreadyInitialized,
//     InvalidInput(String),
//     InsufficientDeposit(String),
// }

// impl fmt::Display for ContractError {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         match self {
//             Self::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
//             Self::InsufficientBalance { available, requested } =>
//                 write!(f, "Insufficient balance: available {}, requested {}", available, requested),
//             Self::SerializationFailed(msg) => write!(f, "Serialization failed: {}", msg),
//             Self::InvalidAccountId(id) => write!(f, "Invalid account ID: {}", id),
//             Self::AlreadyInitialized => write!(f, "Contract has already been initialized"),
//             Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
//             Self::InsufficientDeposit(msg) => env::panic_str(&msg),
//         }
//     }
// }

// impl ContractError {
//     fn panic(&self) -> ! {
//         env::panic_str(&self.to_string())
//     }
// }

// mod config {
//     use near_sdk::Gas;

//     pub const MAIN_ID: &str = "auto-claim-main2.near";
//     pub const BOOSTFARM: &str = "boostfarm.ref-labs.near";
//     pub const REF_FINANCE: &str = "v2.ref-finance.near";
//     pub const BURROW: &str = "contract.main.burrow.near";
//     pub const WRAP: &str = "wrap.near";
//     pub const ORACLE: &str = "priceoracle.near";

//     pub const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(6);
//     pub const GAS_FT_TRANSFER: Gas = Gas::from_tgas(50);
//     pub const GAS_CLAIM_REWARD: Gas = Gas::from_tgas(10);
//     pub const GAS_REMOVE_LIQUIDITY: Gas = Gas::from_tgas(60);
//     pub const GAS_ORACLE_CALL: Gas = Gas::from_tgas(70);
//     pub const GAS_NEAR_DEPOSIT: Gas = Gas::from_tgas(10);
// }

// const NEAR_DECIMALS: u8 = 24;
// const REF_DECIMALS: u8 = 18;
// const SLIPPAGE: u128 = 97;

// #[derive(BorshStorageKey, BorshSerialize)]
// enum StorageKey {
//     Owner,
// }


// #[near(contract_state)]
// #[derive(PanicOnDefault)]
// pub struct ProxyContract {
//     owner: LazyOption<AccountId>,
//     last_compound_call: Timestamp,
//     is_processing: bool,
// }

// #[near]
// impl ProxyContract {
//     #[init]
//     pub fn new() -> Self {
//         let mut owner = LazyOption::new(StorageKey::Owner, None);
//         if owner.get().is_some() {
//             ContractError::AlreadyInitialized.panic();
//         }
//         owner.set(Some(env::predecessor_account_id()));
//         Self {
//             owner,
//             last_compound_call: 0,
//             is_processing: false,
//         }
//     }

//     fn assert_owner(&self) {
//         let predecessor_id = env::predecessor_account_id();
//         let stored_owner = self.owner.get().as_ref().expect("Contract not initialized");
//         let main_id = config::MAIN_ID
//             .parse::<AccountId>()
//             .unwrap_or_else(|_|
//                 ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic()
//             );

//         if predecessor_id != *stored_owner && predecessor_id != main_id {
//             ContractError::Unauthorized(
//                 format!(
//                     "Only owner ({}) or main account ({}) can call this function",
//                     stored_owner,
//                     config::MAIN_ID
//                 )
//             ).panic();
//         }
//     }

//     fn assert_not_processing(&self) {
//         if self.is_processing {
//             env::panic_str("Contract is processing another transaction");
//         }
//     }

//     fn parse_preference(
//         pref: &Value
//     ) -> Option<(String, AccountId, AccountId, bool, String, String)> {
//         let seed_id = pref
//             .get("seed_id")
//             .and_then(|v| v.as_str())
//             .map(String::from)?;
//         let token_id_str = pref.get("token_id").and_then(|v| v.as_str())?;
//         let token_id: AccountId = token_id_str.parse().ok()?;
//         let smart_contract_name_str = pref.get("smart_contract_name").and_then(|v| v.as_str())?;
//         let smart_contract_name: AccountId = smart_contract_name_str.parse().ok()?;
//         let is_active = pref
//             .get("is_active")
//             .and_then(|v| v.as_bool())
//             .unwrap_or(false);
//         let invested_in = pref
//             .get("invested_in")
//             .and_then(|v| v.as_str())
//             .map(String::from)
//             .unwrap_or_else(|| {
//                 env::log_str("Missing or invalid reinvest_to, defaulting to 'Burrow'");
//                 "Burrow".to_string()
//             });
//         let reinvest_to = pref
//             .get("reinvest_to")
//             .and_then(|v| v.as_str())
//             .map(String::from)
//             .unwrap_or_else(|| {
//                 env::log_str("Missing or invalid reinvest_to, defaulting to 'Burrow'");
//                 "Burrow".to_string()
//             });

//         Some((seed_id, token_id, smart_contract_name, is_active, invested_in, reinvest_to))
//     }

//     //This function is used for the old smart contract that uses true as string instead of bool
//     // fn parse_preference(pref: &Value) -> Option<(String, AccountId, AccountId, bool, String, String)> {
//     //     let seed_id = pref.get("seed_id").and_then(|v| v.as_str()).map(String::from)?;
//     //     if seed_id.len() > 64 {
//     //         env::log_str("seed_id exceeds maximum length of 64 characters");
//     //         return None;
//     //     }
//     //     let token_id = pref.get("token_id")
//     //         .and_then(|v| v.as_str())
//     //         .and_then(|s| s.parse().ok())?;
//     //     let smart_contract_name = pref.get("smart_contract_name")
//     //         .and_then(|v| v.as_str())
//     //         .and_then(|s| s.parse().ok())?;

//     //     let is_active = pref.get("is_active")
//     //         .and_then(|v| v.as_str())
//     //         .map(|s| s.to_lowercase() == "true")
//     //         .unwrap_or(false);

//     //     let invested_in = pref
//     //         .get("invested_in")
//     //         .and_then(|v| v.as_str())
//     //         .map(String::from)
//     //         .unwrap_or_else(|| {
//     //             env::log_str("Missing or invalid reinvest_to, defaulting to 'Burrow'");
//     //             "Burrow".to_string()
//     //         });
//     //     let reinvest_to = pref.get("reinvest_to")
//     //         .and_then(|v| v.as_str())
//     //         .map(String::from)
//     //         .unwrap_or_else(|| {
//     //             env::log_str("Missing or invalid reinvest_to, defaulting to 'Burrow'");
//     //             "Burrow".to_string()
//     //         });

//     //     Some((seed_id, token_id, smart_contract_name, is_active, invested_in, reinvest_to))
//     // }

//     pub fn get_owner(&self) -> AccountId {
//         self.owner.get().as_ref().expect("Contract not initialized").clone()
//     }



//     pub fn compound(&mut self) -> Promise {
//         let current_time = env::block_timestamp();
//         assert!(
//             current_time - self.last_compound_call >= 3_600_000_000_000,
//             "Compound can only be called once every hour"
//         );
//         self.assert_not_processing();
//         self.is_processing = true;

//         let caller = env::predecessor_account_id();
//         let owner = self.owner.get().as_ref().expect("Contract not initialized").clone();
//         let main_account_id: AccountId = config::MAIN_ID
//             .parse()
//             .unwrap_or_else(|_|
//                 ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic()
//             );



//         let get_user_promise = Promise::new(main_account_id).function_call(
//             "get_user".to_string(),
//             json!({"wallet_id": owner}).to_string().into_bytes(),
//             NearToken::from_yoctonear(0),
//             Gas::from_tgas(2)
//         );

//         get_user_promise
//             .then(

//                 // self.compound_callback(
//                 //  U128(env::account_balance().as_yoctonear()), caller, pool_id
//                 // )
//                 Promise::new(env::current_account_id()).function_call(
//                     "compound_callback".to_string(),
//                     json!({"pre_claim_balance": U128(env::account_balance().as_yoctonear()), "caller": caller,})
//                         .to_string()
//                         .into_bytes(),
//                     NearToken::from_yoctonear(0),
//                     Gas::from_tgas(285)
//                 )
//             )
//     }

//     #[private]
//     pub fn compound_callback(
//         &mut self,
//         pre_claim_balance: U128,
//         caller: AccountId,
//     ) -> PromiseOrValue<Vec<Promise>> {
//         assert_eq!(
//             env::predecessor_account_id(),
//             env::current_account_id(),
//             "Callback can only be called by the contract itself"
//         );

//         env::log_str(&format!("Promise results count: {}", env::promise_results_count()));
//         if env::promise_results_count() != 1 {
//             self.is_processing = false;
//             env::log_str("Unexpected number of promise results");
//             return PromiseOrValue::Value(vec![]);
//         }

//         let mut all_promises = Vec::new();

      

//         match env::promise_result(0) {
//             PromiseResult::Successful(result) => {
//                 let user_data: Value = serde_json::from_slice(&result).unwrap_or_else(|e| {
//                     self.is_processing = false;
//                     ContractError::SerializationFailed(e.to_string()).panic()
//                 });

//                 let preferences = match user_data.get("preferences").and_then(|v| v.as_array()) {
//                     Some(prefs) => prefs,
//                     None => {
//                         self.is_processing = false;
//                         self.last_compound_call = env::block_timestamp();
//                         env::log_str("No preferences found in user data");
//                         &Vec::new()
//                     }
//                 };

//                 let mut claim_promises = Vec::new();

//                 for pref in preferences {
//                     let (
//                         seed_id,
//                         token_id,
//                         smart_contract_name,
//                         is_active,
//                         invested_in,
//                         reinvest_to,
//                     ) = match Self::parse_preference(pref) {
//                         Some(data) => data,
//                         None => {
//                             continue;
//                         }
//                     };

//                     if !is_active {
//                         continue;
//                     }

//                     if invested_in == "Burrow" {
//                         let claim_from_burrow = self.claim_from_burrow();
                        
//                         // Promise::new(
//                         //     env::current_account_id()
//                         // ).function_call(
//                         //     "claim_from_burrow".to_string(),
//                         //     json!({})
//                         //         .to_string()
//                         //         .into_bytes(),
//                         //     NearToken::from_yoctonear(0),
//                         //     Gas::from_tgas(20)
//                         // );

//                         let handle_promise = claim_from_burrow;

//                         claim_promises.push(handle_promise);
//                     } else {
//                         let claim_all_rewards = self.claim_all_rewards(seed_id, token_id);
                        
//                         // Promise::new(
//                         //     env::current_account_id()
//                         // ).function_call(
//                         //     "claim_all_rewards".to_string(),
//                         //     json!({"seed_id": seed_id, "token_id": token_id})
//                         //         .to_string()
//                         //         .into_bytes(),
//                         //     NearToken::from_yoctonear(0),
//                         //     Gas::from_tgas(20)
//                         // );

//                         let handle_promise = claim_all_rewards;

//                         claim_promises.push(handle_promise);
//                     }
//                 }

//                 if claim_promises.is_empty() {
//                     self.last_compound_call = env::block_timestamp();
//                     self.is_processing = false;
//                     env::log_str("No active preferences to process");
//                     return PromiseOrValue::Value(vec![]);
//                 }

//                 let combined_promise = claim_promises
//                     .into_iter()
//                     .reduce(|acc, promise| acc.and(promise))
//                     .unwrap()
//                     // .then(
//                     //     self.finalize_compound(
//                     //          pre_claim_balance,
//                     //    caller,
//                     //          rate,
//                     //         pool_id,
//                     //          preferences.get(0).and_then(|p| p.get("reinvest_to")).and_then(|v| v.as_str()).unwrap_or("Burrow"),
//                     //          preferences.get(0).and_then(|p| p.get("smart_contract_name")).and_then(|v| v.as_str()).unwrap_or("")
//                     //     )
//                     // );

//                     .then(
//                         Promise::new(env::current_account_id()).function_call(
//                             "finalize_compound".to_string(),
//                             json!({
//                                     "pre_claim_balance": pre_claim_balance,
//                                     "caller": caller,
                                   
//                                     "reinvest_to": preferences.get(0).and_then(|p| p.get("reinvest_to")).and_then(|v| v.as_str()).unwrap_or("Burrow"),
//                                     "smart_contract_name": preferences.get(0).and_then(|p| p.get("smart_contract_name")).and_then(|v| v.as_str()).unwrap_or("")
//                                 })
//                                 .to_string()
//                                 .into_bytes(),
//                             NearToken::from_yoctonear(0),
//                             Gas::from_tgas(180)
//                         )
//                     );

//                 all_promises.push(combined_promise);
//             }
//             PromiseResult::Failed => {
//                 self.is_processing = false;
//                 env::log_str("Failed to retrieve user data");
//             }
//         }

//         PromiseOrValue::Value(all_promises)
//     }

//     #[private]
//     pub fn finalize_compound(
//         &mut self,
//         pre_claim_balance: U128,
//         caller: AccountId,
//         reinvest_to: String,
//         smart_contract_name: String
//     ) -> PromiseOrValue<Vec<Promise>> {
//         let predecessor_id = env::predecessor_account_id();
//         let current_id = env::current_account_id();
//         let owner_id = self.owner.get().as_ref().expect("Contract not initialized");

//         if predecessor_id != current_id && predecessor_id != *owner_id {
//             self.is_processing = false;
//             env::panic_str("Callback can only be called by the contract itself or the owner");
//         }

//         let mut final_promises = Vec::new();
//         let post_claim_balance = env::account_balance().as_yoctonear();
//         let balance_increase = post_claim_balance.saturating_sub(pre_claim_balance.0);

//         env::log_str(
//             &format!(
//                 "Pre-claim: {}, Post-claim: {}, Increase: {}",
//                 pre_claim_balance.0,
//                 post_claim_balance,
//                 balance_increase
//             )
//         );

//         let threshold = 2_000_000_000_000_000_000_000_000;
//         if balance_increase < threshold {
//             let caller_share_near = (balance_increase * 5) / 100;

//             let remaining_balance = (balance_increase * 95) / 100;
          

//             env::log_str(
//                 &format!(
//                     "Reinvesting  for caller {} (from {} yoctoNEAR)",
//                     &caller,
//                     caller_share_near,
//                 )
//             );

//             final_promises.push(
//                 Promise::new(env::current_account_id()).function_call(
//                     "reinvest".to_string(),
//                     json!({
//                             "contract_id": smart_contract_name.parse::<AccountId>().unwrap_or_else(|_| {self.is_processing = false; ContractError::InvalidAccountId(smart_contract_name.clone()).panic()}),
//                             "deposit_amount" : U128(remaining_balance),
//                             "reinvest_to": reinvest_to,
//                         })
//                         .to_string()
//                         .into_bytes(),
//                     NearToken::from_yoctonear(0),
//                     Gas::from_tgas(140)
//                 )
//             );
//             final_promises.push(
//                 Promise::new(caller).transfer(NearToken::from_yoctonear(caller_share_near))
//             );
//         } else {
//             env::log_str(
//                 &format!(
//                     "Claimed amount {} yoctoNEAR below threshold of {} yoctoNEAR",
//                     balance_increase,
//                     threshold
//                 )
//             );
//         }

//         self.is_processing = false;
//         self.last_compound_call = env::block_timestamp();
//         PromiseOrValue::Value(final_promises)
//     }

//     #[private]
//     pub fn reinvest(
//         &mut self,
//         contract_id: AccountId,
//         deposit_amount: U128,
//         reinvest_to: String
//     ) -> Vec<Promise> {
//         let mut promises = Vec::new();

//         let balance = env::account_balance().as_yoctonear();
//         let two_near = 2_000_000_000_000_000_000_000_000;

//         if balance <= two_near {
//             self.is_processing = false;
//             env::log_str(&format!("Insufficient balance for reinvestment: {}", balance));
//             return promises;
//         }

//         if reinvest_to == "Burrow" {
//             let deposit_promise = self.deposit_into_burrow(deposit_amount)
            
//                 .then(
//                     Promise::new(env::current_account_id()).function_call(
//                         "handle_reinvest_result".to_string(),
//                         json!({
//                                 "contract_name": contract_id.to_string(),
//                                 "action": "deposit_into_burrow"
//                             })
//                             .to_string()
//                             .into_bytes(),
//                         NearToken::from_yoctonear(0),
//                         Gas::from_tgas(3)
//                     )
//                 );
//             promises.push(deposit_promise);
//         } 
//         else {
//             self.is_processing = false;
//             ContractError::InvalidInput(
//                 format!("Invalid reinvest_to option: {}", reinvest_to)
//             ).panic();
//         }

//         promises
//     }

//     #[private]
//     pub fn handle_reinvest_result(
//         &self,
//         contract_name: String,
//         action: String
//     ) -> PromiseOrValue<()> {
//         if env::promise_results_count() > 0 {
//             match env::promise_result(0) {
//                 PromiseResult::Successful(_) => {
//                     env::log_str(&format!("{} succeeded for {}", action, contract_name));
//                 }
//                 PromiseResult::Failed => {
//                     env::log_str(
//                         &format!(
//                             "Failed to execute {} on {}: Check account registration or balance",
//                             action,
//                             contract_name
//                         )
//                     );
//                 }
//             }
//         } else {
//             env::log_str("No promise result available");
//         }
//         PromiseOrValue::Value(())
//     }

//     #[payable]
//     pub fn stake_lp_tokens(
//         &mut self,
//         pool_id: String,
//         lp_token_amount: U128,
//         user_account: AccountId
//     ) -> Promise {
//         self.assert_owner();

//         if pool_id.is_empty() {
//             ContractError::InvalidInput("pool_id cannot be empty".to_string()).panic();
//         }
//         if lp_token_amount.0 == 0 {
//             ContractError::InvalidInput("lp_token_amount must be non-zero".to_string()).panic();
//         }

//         let min_deposit = NearToken::from_yoctonear(1);
//         let attached = env::attached_deposit();
//         if attached < min_deposit {
//             ContractError::InsufficientDeposit(
//                 format!(
//                     "Attached deposit {} NEAR is less than required {} NEAR",
//                     attached.as_near(),
//                     min_deposit.as_near()
//                 )
//             ).panic();
//         }

//         let transfer_promise = Promise::new(config::REF_FINANCE.parse().unwrap())
//             .function_call(
//                 "mft_transfer_call".to_string(),
//                 json!({
//                 "receiver_id": config::BOOSTFARM,
//                 "token_id": pool_id,
//                 "amount": lp_token_amount,
//                 "msg": "\"Free\""
//             })
//                     .to_string()
//                     .into_bytes(),
//                 NearToken::from_yoctonear(1),
//                 config::GAS_FT_TRANSFER
//             );
//             // .then(
//             //     Self::ext(env::current_account_id())
//             //         .with_static_gas(Gas::from_tgas(100))
//             //         .stake_lp_tokens_callback(pool_id, lp_token_amount, user_account)
//             // );

//         transfer_promise
//     }



//     pub fn unstake_lp(
//         &mut self,
//         seed_id: String,
//         withdraw_amount: U128,
//         token_id: AccountId
//     ) -> Promise {
//         self.assert_owner();
//         if seed_id.is_empty() {
//             ContractError::InvalidInput("seed_id cannot be empty".to_string()).panic();
//         }
//         if withdraw_amount.0 == 0 {
//             ContractError::InvalidInput("withdraw_amount must be non-zero".to_string()).panic();
//         }

//         let unlock_and_withdraw_seed = Promise::new(
//             config::BOOSTFARM
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
//                 )
//         ).function_call(
//             "unlock_and_withdraw_seed".to_string(),
//             json!({
//                 "seed_id": seed_id,
//                 "unlock_amount": "0",
//                 "withdraw_amount": withdraw_amount,
//             })
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(1),
//             config::GAS_FT_TRANSFER
//         );

//         let withdraw_reward_token = Promise::new(
//             config::BOOSTFARM
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
//                 )
//         ).function_call(
//             "withdraw_reward".to_string(),
//             json!({"token_id": token_id}).to_string().into_bytes(),
//             NearToken::from_yoctonear(0),
//             config::GAS_CLAIM_REWARD
//         );

//         let delete_preference = Promise::new(
//             config::MAIN_ID
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic()
//                 )
//         ).function_call(
//             "delete_preference".to_string(),
//             json!({"seed_id": seed_id}).to_string().into_bytes(),
//             NearToken::from_yoctonear(0),
//             Gas::from_tgas(20)
//         );

//         unlock_and_withdraw_seed.then(withdraw_reward_token).then(delete_preference)
//     }

//     #[private]
//     pub fn claim_all_rewards(&mut self, seed_id: String, token_id: AccountId) -> Promise {
//         if seed_id.is_empty() {
//             ContractError::InvalidInput("seed_id cannot be empty".to_string()).panic();
//         }

//         let claim_reward = Promise::new(
//             config::BOOSTFARM
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic()
//                 )
//         ).function_call(
//             "claim_reward_by_seed".to_string(),
//             json!({"seed_id": seed_id, "token_id": token_id}).to_string().into_bytes(),
//             NearToken::from_yoctonear(0),
//             Gas::from_tgas(6)
//         );

//         claim_reward
//     }



//     #[payable]
//     pub fn deposit_into_burrow(&mut self, deposit_amount: U128) -> Promise {
//         if deposit_amount.0 == 0 {
//             ContractError::InvalidInput("Deposit amount must be non-zero".to_string()).panic();
//         }

//         let min_deposit = NearToken::from_yoctonear(deposit_amount.0);
//         let attached = env::attached_deposit();
//         if attached < min_deposit {
//             ContractError::InsufficientDeposit(
//                 format!(
//                     "Attached deposit {} NEAR is less than required {} NEAR",
//                     attached.as_near(),
//                     min_deposit.as_near()
//                 )
//             ).panic();
//         }

//         let burrow_account: AccountId = config::BURROW
//             .parse()
//             .unwrap_or_else(|_| {
//                 ContractError::InvalidAccountId(config::BURROW.to_string()).panic()
//             });
//         let wrap_account: AccountId = config::WRAP
//             .parse()
//             .unwrap_or_else(|_| {
//                 ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
//             });

//         let deposit_promise = Promise::new(wrap_account.clone()).function_call(
//             "near_deposit".to_string(),
//             json!({})
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(deposit_amount.0),
//             config::GAS_NEAR_DEPOSIT
//         );

//         let collateral_transfer = Promise::new(wrap_account.clone()).function_call(
//             "ft_transfer_call".to_string(),
//             json!({
//                     "receiver_id": config::BURROW,
//                     "amount": deposit_amount,
//                     "msg": json!({"Execute": {"actions": [{"IncreaseCollateral": {"token_id": config::WRAP, "max_amount": deposit_amount}}]}}).to_string()
//                 })
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(1),
//             Gas::from_tgas(10)
//         );

//         deposit_promise
//             .then(collateral_transfer)
//             // .then(
//             //     Self::ext(env::current_account_id())
//             //         .with_static_gas(Gas::from_tgas(32))
//             //         .deposit_into_burrow_callback(
//             //             deposit_amount.0.to_string(),
//             //             burrow_account,
//             //             wrap_account
//             //         )
//             // )
//     }



//     pub fn deposit_into_burrow_pool(
//         &mut self,
//         token_id: AccountId,
//         deposit_amount: U128
//     ) -> Promise {
//         if deposit_amount.0 == 0 {
//             ContractError::InvalidInput("deposit_amount must be non-zero".to_string()).panic();
//         }

//         Promise::new(token_id.clone()).function_call(
//             "ft_transfer_call".to_string(),
//             json!({
//                 "receiver_id": config::BURROW,
//                 "amount": deposit_amount,
//                 "msg": json!({"Execute": {"actions": [{"IncreaseCollateral": {"token_id": token_id, "max_amount": deposit_amount}}]}}).to_string()
//             })
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(1),
//             config::GAS_FT_TRANSFER
//         )
//     }

//     pub fn claim_from_burrow(&mut self) -> Promise {
//         Promise::new(
//             config::BURROW
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::BURROW.to_string()).panic()
//                 )
//         ).function_call(
//             "account_farm_claim_all".to_string(),
//             json!({})
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(0),
//             config::GAS_CLAIM_REWARD
//         )
//     }

//     pub fn withdraw_from_borrow_pool(&mut self, withdraw_amount: U128) -> Promise {
//         self.assert_owner();
//         if withdraw_amount.0 == 0 {
//             ContractError::InvalidInput("withdraw_amount must be non-zero".to_string()).panic();
//         }

//         let wrap_promise = Promise::new(
//             config::ORACLE
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::ORACLE.to_string()).panic()
//                 )
//         ).function_call(
//             "oracle_call".to_string(),
//             json!({
//                 "receiver_id": config::BURROW,
//                 "msg": json!({"Execute": {"actions": [
//                     {"DecreaseCollateral": {"token_id": config::WRAP}},
//                     {"Withdraw": {"token_id": config::WRAP}}
//                 ]}}).to_string()
//             })
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(1),
//             config::GAS_ORACLE_CALL
//         );

//         let wrap_promise2 = Promise::new(
//             config::WRAP
//                 .parse()
//                 .unwrap_or_else(|_|
//                     ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
//                 )
//         ).function_call(
//             "near_withdraw".to_string(),
//             json!({"amount": withdraw_amount}).to_string().into_bytes(),
//             NearToken::from_yoctonear(1),
//             config::GAS_FT_TRANSFER
//         );

//         wrap_promise.then(wrap_promise2)
//     }

//     pub fn withdraw_token(
//         &mut self,
//         token_id: AccountId,
//         receiver_id: AccountId,
//         amount: U128
//     ) -> Promise {
//         self.assert_owner();
//         let attached_deposit = env::attached_deposit();
//         if attached_deposit != NearToken::from_yoctonear(1) {
//             env::panic_str("Requires exactly 1 yoctoNEAR attached deposit");
//         }

//         if amount.0 == 0 {
//             ContractError::InvalidInput("amount must be non-zero".to_string()).panic();
//         }

//         Promise::new(token_id.clone()).function_call(
//             "ft_transfer".to_string(),
//             json!({
//                 "receiver_id": receiver_id,
//                 "amount": amount,
//                 "memo": "Withdraw token from contract"
//             })
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(1),
//             config::GAS_FT_TRANSFER
//         )
//     }

//     pub fn get_contract_balance(&self) -> NearToken {
//         self.assert_owner();
//         env::account_balance()
//     }

//     #[payable]
//     pub fn withdraw_amount(&mut self, beneficiary: AccountId, amount: U128) -> Promise {
//         self.assert_owner();

//         let attached_deposit = env::attached_deposit();
//         if attached_deposit != NearToken::from_yoctonear(1) {
//             env::panic_str("Requires exactly 1 yoctoNEAR attached deposit");
//         }

//         if amount.0 == 0 {
//             ContractError::InvalidInput("amount must be non-zero".to_string()).panic();
//         }

//         let available_balance = env::account_balance();
//         let storage_cost = (env::storage_usage() as u128) * env::storage_byte_cost().as_yoctonear();
//         let total_balance = available_balance.as_yoctonear() + storage_cost;
//         let amount_near = NearToken::from_yoctonear(amount.0);

//         if available_balance < amount_near {
//             env::log_str(
//                 &format!(
//                     "Total balance: {}, Storage reserved: {}, Available: {}",
//                     total_balance,
//                     storage_cost,
//                     available_balance.as_yoctonear()
//                 )
//             );
//             (ContractError::InsufficientBalance {
//                 available: available_balance.as_yoctonear(),
//                 requested: amount_near.as_yoctonear(),
//             }).panic();
//         }

//         env::log_str(
//             &format!(
//                 "Initiating transfer of {} yoctoNEAR to beneficiary: {}",
//                 amount.0,
//                 beneficiary
//             )
//         );
//         Promise::new(beneficiary).transfer(amount_near)
//     }

//     pub fn transfer_lp_token(
//         &mut self,
//         receiver_id: AccountId,
//         token_id: String,
//         amount: U128
//     ) -> Promise {
//         self.assert_owner();

//         if token_id.is_empty() {
//             ContractError::InvalidInput("Token ID cannot be empty".to_string()).panic();
//         }
//         if amount.0 == 0 {
//             ContractError::InvalidInput("Amount must be non-zero".to_string()).panic();
//         }

//         let min_deposit = NearToken::from_yoctonear(1);
//         let attached = env::attached_deposit();
//         if attached < min_deposit {
//             ContractError::InsufficientDeposit(
//                 format!(
//                     "Attached deposit {} NEAR is less than required {} NEAR",
//                     attached.as_near(),
//                     min_deposit.as_near()
//                 )
//             ).panic();
//         }

//         let account_id = env::current_account_id();

//         Promise::new(receiver_id).function_call(
//             "mft_transfer".to_string(),
//             json!({
//                     "receiver_id": account_id.to_string(),
//                     "token_id": token_id,
//                     "amount": amount,
//                     "memo": null
//                 })
//                 .to_string()
//                 .into_bytes(),
//             NearToken::from_yoctonear(1),
//             Gas::from_tgas(85)
//         )
//     }
// }

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use near_sdk::test_utils::{ VMContextBuilder, accounts };
//     use near_sdk::testing_env;

//     #[test]
//     fn test_new() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let contract = ProxyContract::new();
//         assert_eq!(contract.get_owner(), accounts(0));
//     }

//     #[test]
//     fn test_assert_owner_success() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let contract = ProxyContract::new();
//         contract.assert_owner();
//     }

//     #[test]
//     fn test_assert_owner_main_id_success() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(config::MAIN_ID.parse().unwrap())
//             .build();
//         testing_env!(context);
//         let contract = ProxyContract::new();
//         contract.assert_owner();
//     }

//     #[test]
//     #[should_panic(expected = "Unauthorized")]
//     fn test_assert_owner_failure() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let contract = ProxyContract::new();
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(1)).build();
//         testing_env!(context);
//         contract.assert_owner();
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: pool_id cannot be empty")]
//     fn test_stake_lp_tokens_empty_pool_id() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(1))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.stake_lp_tokens("".to_string(), U128(1_000), accounts(1));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: lp_token_amount must be non-zero")]
//     fn test_stake_lp_tokens_zero_amount() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(1))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.stake_lp_tokens("pool1".to_string(), U128(0), accounts(1));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
//     fn test_stake_x_ref_zero_deposit() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.stake_x_ref(accounts(1), U128(0), U128(0), "123".to_string());
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: wnear_to_ref_pool_id cannot be empty")]
//     fn test_stake_x_ref_empty_pool_id() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.stake_x_ref(accounts(1), U128(1_000), U128(0), "".to_string());
//     }

//     #[test]
//     fn test_reinvest_insufficient_balance() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .account_balance(NearToken::from_yoctonear(1_000_000_000_000_000_000_000_000))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         let promises = contract.reinvest(
//             accounts(1),
//             U128(0),
//             U128(0),
//             "123".to_string(),
//             "Burrow".to_string()
//         );
//         assert_eq!(promises.len(), 0);
//     }

//     #[test]
//     fn test_reinvest_valid_burrow() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .account_balance(NearToken::from_yoctonear(3_000_000_000_000_000_000_000_000))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         let promises = contract.reinvest(
//             accounts(1),
//             U128(1_000),
//             U128(1_000_000_000_000_000_000_000_000),
//             "123".to_string(),
//             "Burrow".to_string()
//         );
//         assert_eq!(promises.len(), 1);
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: Invalid reinvest_to option")]
//     fn test_reinvest_invalid_option() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .account_balance(NearToken::from_yoctonear(3_000_000_000_000_000_000_000_000))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.reinvest(
//             accounts(1),
//             U128(1_000),
//             U128(1_000),
//             "123".to_string(),
//             "Invalid".to_string()
//         );
//     }

//     #[test]
//     #[should_panic(expected = "Requires exactly 1 yoctoNEAR attached deposit")]
//     fn test_withdraw_amount_invalid_deposit() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(0))
//             .account_balance(NearToken::from_near(5))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.withdraw_amount(accounts(1), U128(1_000_000_000_000_000_000_000_000));
//     }

//     #[test]
//     #[should_panic(expected = "Insufficient balance")]
//     fn test_withdraw_amount_insufficient_balance() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(1))
//             .account_balance(NearToken::from_near(1))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.withdraw_amount(accounts(1), U128(2_000_000_000_000_000_000_000_000));
//     }

//     #[test]
//     fn test_withdraw_amount_success() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(1))
//             .account_balance(NearToken::from_near(5))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         let _promise = contract.withdraw_amount(
//             accounts(1),
//             U128(1_000_000_000_000_000_000_000_000)
//         );
//     }

//     #[test]
//     #[should_panic(expected = "Compound can only be called once every hour")]
//     fn test_compound_too_soon() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .block_timestamp(0)
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.compound(123);

//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .block_timestamp(1_000_000_000_000)
//             .build();
//         testing_env!(context);
//         contract.compound(123);
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: seed_id cannot be empty")]
//     fn test_unstake_lp_empty_seed_id() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.unstake_lp("".to_string(), U128(1_000), accounts(1));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: withdraw_amount must be non-zero")]
//     fn test_unstake_lp_zero_amount() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.unstake_lp("seed1".to_string(), U128(0), accounts(1));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: seed_id cannot be empty")]
//     fn test_claim_all_rewards_empty_seed_id() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.claim_all_rewards("".to_string(), accounts(1));
//     }

//     // #[test]
//     // #[should_panic(expected = "Invalid input: shares must be non-zero")]
//     // fn test_remove_liquidity_zero_shares() {
//     //     let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//     //     testing_env!(context);
//     //     let mut contract = ProxyContract::new();
//     //     contract.remove_liquidity_and_withdraw_tokens(
//     //         123,
//     //         U128(0),
//     //         U128(0),
//     //         U128(0),
//     //         accounts(1),
//     //         accounts(2)
//     //     );
//     // }

//     #[test]
//     #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
//     fn test_deposit_into_burrow_zero_amount() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.deposit_into_burrow(U128(0));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
//     fn test_deposit_into_burrow_pool_zero_amount() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.deposit_into_burrow_pool(accounts(1), U128(0));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: withdraw_amount must be non-zero")]
//     fn test_withdraw_from_borrow_pool_zero_amount() {
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(0)).build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.withdraw_from_borrow_pool(U128(0));
//     }

//     #[test]
//     #[should_panic(expected = "Requires exactly 1 yoctoNEAR attached deposit")]
//     fn test_withdraw_token_invalid_deposit() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(0))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.withdraw_token(accounts(1), accounts(2), U128(1_000));
//     }

//     #[test]
//     #[should_panic(expected = "Invalid input: amount must be non-zero")]
//     fn test_withdraw_token_zero_amount() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .attached_deposit(NearToken::from_yoctonear(1))
//             .build();
//         testing_env!(context);
//         let mut contract = ProxyContract::new();
//         contract.withdraw_token(accounts(1), accounts(2), U128(0));
//     }

//     #[test]
//     fn test_get_contract_balance() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .account_balance(NearToken::from_near(5))
//             .build();
//         testing_env!(context);
//         let contract = ProxyContract::new();
//         let balance = contract.get_contract_balance();
//         assert_eq!(balance, NearToken::from_near(5));
//     }

//     #[test]
//     #[should_panic(expected = "Unauthorized")]
//     fn test_get_contract_balance_unauthorized() {
//         let context = VMContextBuilder::new()
//             .predecessor_account_id(accounts(0))
//             .account_balance(NearToken::from_near(5))
//             .build();
//         testing_env!(context);
//         let contract = ProxyContract::new();
//         let context = VMContextBuilder::new().predecessor_account_id(accounts(1)).build();
//         testing_env!(context);
//         contract.get_contract_balance();
//     }
// }
