use near_sdk::{
    near,
    env,
    NearToken,
    Promise,
    AccountId,
    BorshStorageKey,
    PanicOnDefault,
    Timestamp,
    borsh::{ self, BorshSerialize, BorshDeserialize },
    store::LazyOption,
    json_types::U128,
    Gas,
    PromiseOrValue,
    PromiseResult,
};
use serde::{ Serialize, Deserialize };
use serde_json::{ json, Value };
use core::fmt;
use schemars::JsonSchema;
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

    pub const MAIN_ID: &str = "compoundx.near";
    pub const BOOSTFARM: &str = "boostfarm.ref-labs.near";
    pub const REF_FINANCE: &str = "v2.ref-finance.near";
    pub const BURROW: &str = "contract.main.burrow.near";
    pub const WRAP: &str = "wrap.near";
    pub const ORACLE: &str = "priceoracle.near";

    pub const GAS_FT_TRANSFER: Gas = Gas::from_tgas(50);
    pub const GAS_CLAIM_REWARD: Gas = Gas::from_tgas(10);
    pub const GAS_ORACLE_CALL: Gas = Gas::from_tgas(70);
    pub const GAS_NEAR_DEPOSIT: Gas = Gas::from_tgas(10);
}

#[derive(BorshStorageKey, BorshSerialize)]
enum StorageKey {
    Owner,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ReinvestFrom {
    BurrowFarm {
        seed_id: String,
        token_id: AccountId,
    },
    RefBoostfarm {
        seed_id: String,
        pool_id: u64,
    },
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ReinvestTo {
    #[schemars(with = "String")] BurrowMainVault {
        seed_id: String,
        token_id: AccountId,
    },
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
    pub fn new(user: AccountId) -> Self {
        let mut owner = LazyOption::new(StorageKey::Owner, None);
        if owner.get().is_some() {
            ContractError::AlreadyInitialized.panic();
        }

        owner.set(Some(user));
        Self {
            owner,
            last_compound_call: 0,
            is_processing: false,
        }
    }

    // Function to create owner when  sub-account contract is deployed  within another contract
    // pub fn new() -> Self {
    //     let mut owner = LazyOption::new(StorageKey::Owner, None);
    //     if owner.get().is_some() {
    //         ContractError::AlreadyInitialized.panic();
    //     }
    //     owner.set(Some(env::predecessor_account_id()));
    //     Self {
    //         owner,
    //         last_compound_call: 0,
    //         is_processing: false,
    //     }
    // }

    fn assert_owner(&self) {
        let predecessor_id = env::predecessor_account_id();
        let stored_owner = self.owner.get().as_ref().expect("Contract not initialized");

        if predecessor_id != *stored_owner {
            ContractError::Unauthorized(
                format!("Only owner ({})  can call this function", stored_owner)
            ).panic();
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
        let smart_contract_name = pref
            .get("smart_contract_name")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<AccountId>().ok())?;
        let is_active = pref
            .get("is_active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let invested_in_obj = pref.get("invested_in")?;
        let (invested_in, seed_id) = if let Some(burrow) = invested_in_obj.get("Burrow") {
            let seed_id = burrow
                .get("seed_id")
                .and_then(|v| v.as_str())
                .map(String::from)?;
            let token_id = burrow
                .get("token_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<AccountId>().ok())?;
            (ReinvestFrom::BurrowFarm { seed_id: seed_id.clone(), token_id }, seed_id)
        } else if let Some(seed_id) = invested_in_obj.get("RefBoostfarm") {
            let seed_id = seed_id.as_str().map(String::from)?;
            let pool_id = seed_id.parse::<u64>().ok()?;
            (ReinvestFrom::RefBoostfarm { seed_id: seed_id.clone(), pool_id }, seed_id)
        } else {
            return None;
        };

        let reinvest_to_obj = pref.get("reinvest_to")?;
        let reinvest_to = if let Some(burrow) = reinvest_to_obj.get("Burrow") {
            let seed_id = burrow
                .get("seed_id")
                .and_then(|v| v.as_str())
                .map(String::from)?;
            let token_id = burrow
                .get("token_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<AccountId>().ok())?;
            ReinvestTo::BurrowMainVault { seed_id, token_id }
        } else {
            return None;
        };

        let token_id = match &invested_in {
            ReinvestFrom::BurrowFarm { token_id, .. } => token_id.clone(),
            ReinvestFrom::RefBoostfarm { .. } => smart_contract_name.clone(),
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
            .unwrap_or_else(|_|
                ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic()
            );

        let get_user_promise = Promise::new(main_account_id).function_call(
            "get_user".to_string(),
            json!({"wallet_id": owner}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(2)
        );

        get_user_promise.then(
            Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(285))
                .with_attached_deposit(NearToken::from_yoctonear(0))
                .compound_callback(U128(env::account_balance().as_yoctonear()), caller)
        )
    }

    #[private]
    pub fn compound_callback(
        &mut self,
        pre_claim_balance: U128,
        caller: AccountId
    ) -> PromiseOrValue<Promise> {
        env::log_str(&format!("Promise results count: {}", env::promise_results_count()));
        if env::promise_results_count() != 1 {
            self.is_processing = false;
            env::log_str("Unexpected number of promise results");
            return PromiseOrValue::Value(Promise::new(env::current_account_id()));
        }

        match env::promise_result(0) {
            PromiseResult::Successful(result) => {
                let raw_data_str = String::from_utf8_lossy(&result).to_string();
                env::log_str(&format!("Raw user data: {}", raw_data_str));

                let user_data: Value = serde_json::from_slice(&result).unwrap_or_else(|e| {
                    self.is_processing = false;
                    ContractError::SerializationFailed(e.to_string()).panic()
                });

                let preference = user_data.get("preference");
                env::log_str(&format!("Preference: {:?}", preference));

                let active_pref = match preference {
                    Some(pref) if pref != &Value::Null => {
                        let parsed = Self::parse_preference(pref);
                        if
                            let Some(
                                (
                                    seed_id,
                                    token_id,
                                    smart_contract_name,
                                    is_active,
                                    invested_in,
                                    reinvest_to,
                                ),
                            ) = parsed
                        {
                            if is_active {
                                env::log_str("Found active preference");
                                Some((
                                    seed_id,
                                    token_id,
                                    smart_contract_name,
                                    invested_in,
                                    reinvest_to,
                                ))
                            } else {
                                env::log_str("Preference is inactive");
                                None
                            }
                        } else {
                            env::log_str("Preference failed to parse");
                            None
                        }
                    }
                    _ => {
                        env::log_str("No preference or preference is null");
                        None
                    }
                };

                match active_pref {
                    Some((seed_id, _token_id, smart_contract_name, invested_in, reinvest_to)) => {
                        let promise = match invested_in {
                            ReinvestFrom::BurrowFarm { .. } => self.claim_from_burrow(),
                            ReinvestFrom::RefBoostfarm { .. } => {
                                self.claim_all_rewards(seed_id, smart_contract_name.clone())
                            }
                        };
                        PromiseOrValue::Promise(
                            promise.then(
                                Self::ext(env::current_account_id())
                                    .with_static_gas(Gas::from_tgas(180))
                                    .with_attached_deposit(NearToken::from_yoctonear(0))
                                    .finalize_compound(pre_claim_balance, caller, reinvest_to)
                            )
                        )
                    }
                    None => {
                        self.is_processing = false;
                        self.last_compound_call = env::block_timestamp();
                        env::log_str("No active preference to process");
                        PromiseOrValue::Value(Promise::new(env::current_account_id()))
                    }
                }
            }
            PromiseResult::Failed => {
                self.is_processing = false;
                env::log_str("Failed to retrieve user data");
                PromiseOrValue::Value(Promise::new(env::current_account_id()))
            }
        }
    }

    #[private]
    #[allow(private_interfaces)]
    pub fn finalize_compound(
        &mut self,
        pre_claim_balance: U128,
        caller: AccountId,
        reinvest_to: ReinvestTo
    ) -> PromiseOrValue<Promise> {
        let predecessor_id = env::predecessor_account_id();
        let current_id = env::current_account_id();
        let owner_id = self.owner.get().as_ref().expect("Contract not initialized");

        if predecessor_id != current_id && predecessor_id != *owner_id {
            self.is_processing = false;
            env::panic_str("Callback can only be called by the contract itself or the owner");
        }

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

        let threshold = 20_000_000_000_000//2_000_000_000_000_000_000_000_000;
        if balance_increase >= threshold {
            let caller_share_near = (balance_increase * 5) / 100;
            let remaining_balance = (balance_increase * 95) / 100;

            env::log_str(
                &format!(
                    "Reinvesting {} yoctoNEAR for caller {} (from {} yoctoNEAR)",
                    remaining_balance,
                    caller,
                    caller_share_near
                )
            );

            let contract_id = match reinvest_to {
                ReinvestTo::BurrowMainVault { .. } => {
                    config::BURROW.parse::<AccountId>().unwrap()
                }
            };

            let reinvest_promise = Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(140))
                .with_attached_deposit(NearToken::from_yoctonear(0))
                .reinvest(contract_id, U128(remaining_balance), reinvest_to);

            let transfer_promise = Promise::new(caller).transfer(
                NearToken::from_yoctonear(caller_share_near)
            );

            PromiseOrValue::Promise(reinvest_promise.then(transfer_promise))
        } else {
            self.is_processing = false;
            env::log_str(
                &format!(
                    "Claimed amount {} yoctoNEAR below threshold of {} yoctoNEAR",
                    balance_increase,
                    threshold
                )
            );
            PromiseOrValue::Value(Promise::new(env::current_account_id()))
        }
    }

    #[private]
    #[allow(private_interfaces)]
    pub fn reinvest(
        &mut self,
        contract_id: AccountId,
        deposit_amount: U128,
        reinvest_to: ReinvestTo
    ) -> Promise {
        let balance = env::account_balance().as_yoctonear();
        let min_balance = NearToken::from_near(2);

        if balance <= min_balance.as_yoctonear() {
            self.is_processing = false;
            env::log_str(&format!("Insufficient balance for reinvestment: {}", balance));
            return Promise::new(env::current_account_id());
        }

        match reinvest_to {
            ReinvestTo::BurrowMainVault { .. } => {
                self.deposit_into_burrow(deposit_amount).then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(3))
                        .with_attached_deposit(NearToken::from_yoctonear(0))
                        .handle_reinvest_result(
                            contract_id.to_string(),
                            "deposit_into_burrow".to_string()
                        )
                )
            }
        }
    }

    fn claim_from_burrow(&mut self) -> Promise {
        Promise::new(config::BURROW.parse().unwrap()).function_call(
            "account_farm_claim_all".to_string(),
            json!({})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD
        )
    }

    fn claim_all_rewards(&mut self, seed_id: String, token_id: AccountId) -> Promise {
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

    fn deposit_into_burrow(&mut self, deposit_amount: U128) -> Promise {
        if deposit_amount.0 == 0 {
            ContractError::InvalidInput("Deposit amount must be non-zero".to_string()).panic();
        }

        let wrap_account: AccountId = config::WRAP
            .parse()
            .unwrap_or_else(|_| {
                ContractError::InvalidAccountId(config::WRAP.to_string()).panic()
            });

        let deposit_promise = Promise::new(wrap_account.clone()).function_call(
            "near_deposit".to_string(),
            json!({})
                .to_string()
                .into_bytes(),
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
        self.assert_owner();
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
    pub fn stake_lp_tokens(&mut self, pool_id: String, lp_token_amount: U128) -> Promise {
        self.assert_owner();

        if pool_id.is_empty() {
            ContractError::InvalidInput("pool_id cannot be empty".to_string()).panic();
        }
        if lp_token_amount.0 == 0 {
            ContractError::InvalidInput("lp_token_amount must be non-zero".to_string()).panic();
        }

        let attached = env::attached_deposit();
        if attached.is_zero() {
            ContractError::InsufficientDeposit(
                format!(
                    "Attached deposit {} NEAR is required (must be non-zero)",
                    attached.as_near()
                )
            ).panic();
        }

        let transfer_promise = Promise::new(config::REF_FINANCE.parse().unwrap()).function_call(
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

    #[payable]
    pub fn withdraw_from_borrow_pool(
        &mut self,
        withdraw_amount: U128,
        token_id: AccountId
    ) -> Promise {
        self.assert_owner();

        if withdraw_amount.0 == 0 {
            ContractError::InvalidInput("withdraw_amount must be non-zero".to_string()).panic();
        }

        // Step 1: Withdraw from Burrow
        let withdraw_promise = Promise::new(
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
                    {"DecreaseCollateral": {"token_id": token_id.to_string()}},
                    {"Withdraw": {"token_id": token_id.to_string()}}
                ]}}).to_string()
            })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_ORACLE_CALL
        );

        if token_id == "wrap.near" {
            withdraw_promise.then(
                Promise::new(token_id).function_call(
                    "near_withdraw".to_string(),
                    json!({"amount": withdraw_amount}).to_string().into_bytes(),
                    NearToken::from_yoctonear(1),
                    config::GAS_FT_TRANSFER
                )
            )
        } else {
            withdraw_promise
        }
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

        let attached = env::attached_deposit();
        if attached.is_zero() {
            ContractError::InsufficientDeposit(
                format!(
                    "Attached deposit {} NEAR is required (must be non-zero)",
                    attached.as_near()
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
    use near_sdk::test_utils::{ VMContextBuilder, accounts };
    use near_sdk::{ testing_env };
    use near_sdk::json_types::U128;
    // use near_sdk::serde_json::{self, json};
    // use near_sdk::PromiseResult;
    // use near_sdk::AccountId;

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
        contract.stake_lp_tokens("".to_string(), U128(1_000));
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
        contract.stake_lp_tokens("pool1".to_string(), U128(0));
    }

    #[test]
    #[should_panic(expected = "Invalid input: Deposit amount must be non-zero")]
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
        contract.withdraw_from_borrow_pool(U128(0), "usdc.example.testnet".parse().unwrap());
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
}
