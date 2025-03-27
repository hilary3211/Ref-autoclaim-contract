use near_sdk::{
    near, env, NearToken, Promise, AccountId, BorshStorageKey, PanicOnDefault, Timestamp,
    borsh::{self, BorshSerialize},
    store::LazyOption,
    json_types::U128,
    Gas, PromiseOrValue, PromiseResult,
};
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
    
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
            Self::InsufficientBalance { available, requested } => write!(
                f,
                "Insufficient balance: available {}, requested {}",
                available, requested
            ),
            Self::SerializationFailed(msg) => write!(f, "Serialization failed: {}", msg),
            Self::InvalidAccountId(id) => write!(f, "Invalid account ID: {}", id),
            Self::AlreadyInitialized => write!(f, "Contract has already been initialized"),
            Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            
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

    pub const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(50);
    pub const GAS_FT_TRANSFER: Gas = Gas::from_tgas(50);
    pub const GAS_CLAIM_REWARD: Gas = Gas::from_tgas(30);
    pub const GAS_REMOVE_LIQUIDITY: Gas = Gas::from_tgas(60);
    pub const GAS_ORACLE_CALL: Gas = Gas::from_tgas(70);
    pub const GAS_NEAR_DEPOSIT: Gas = Gas::from_tgas(50);
}

#[derive(BorshStorageKey, BorshSerialize)]
enum StorageKey {
    Owner,
   
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct ProxyContract {
    owner: LazyOption<AccountId>,
    last_compound_call: Timestamp,
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

    pub fn get_owner(&self) -> AccountId {
        self.owner.get().as_ref().expect("Contract not initialized").clone()
    }

    #[private]
    pub fn stake_x_ref(&mut self, smart_contract_name: AccountId, deposit_amount: U128, receiver_id: AccountId, min_amount_out: U128, pool_id: String) -> Promise {
        if deposit_amount.0 == 0 {
            ContractError::InvalidInput("deposit_amount must be non-zero".to_string()).panic();
        }
        if pool_id.is_empty() {
            ContractError::InvalidInput("pool_id cannot be empty".to_string()).panic();
        }

        let p_0 = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::WRAP.to_string()).panic())
        )
        .function_call(
            "near_deposit".to_string(),
            "{}".to_string().into_bytes(),
            NearToken::from_yoctonear(deposit_amount.0),
            config::GAS_NEAR_DEPOSIT,
        );

        let p_1 = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::WRAP.to_string()).panic())
        )
        .function_call(
            "storage_deposit".to_string(),
            json!({
                "account_id": smart_contract_name,
                "registration_only": true,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_millinear(125),
            config::GAS_STORAGE_DEPOSIT,
        );

        let p_2 = Promise::new(
            "token.v2.ref-finance.near"
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId("token.v2.ref-finance.near".to_string()).panic())
        )
        .function_call(
            "storage_deposit".to_string(),
            json!({
                "account_id": smart_contract_name,
                "registration_only": true,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_millinear(125),
            config::GAS_STORAGE_DEPOSIT,
        );

        let p_3 = Promise::new(
            "xtoken.ref-finance.near"
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId("xtoken.ref-finance.near".to_string()).panic())
        )
        .function_call(
            "storage_deposit".to_string(),
            json!({
                "account_id": smart_contract_name,
                "registration_only": true,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_millinear(125),
            config::GAS_STORAGE_DEPOSIT,
        );

        let p_4 = Promise::new(
            "token.v2.ref-finance.near"
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId("token.v2.ref-finance.near".to_string()).panic())
        )
        .function_call(
            "ft_transfer_call".to_string(),
            json!({
                "receiver_id": receiver_id,
                "amount": deposit_amount,
                "msg": json!({
                    "force": 0,
                    "actions": [
                        {
                            "pool_id": pool_id,
                            "token_in": config::WRAP,
                            "token_out": "xtoken.ref-finance.near",
                            "amount_in": deposit_amount,
                            "min_amount_out": min_amount_out,
                        }
                    ]
                }).to_string()
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER,
        );

        p_0.and(p_1).and(p_2).and(p_3).then(p_4)
    }

    pub fn compound(&mut self) -> Vec<Promise> {
        if self.owner.get().is_none() {
            ContractError::Unauthorized("Contract not initialized".to_string()).panic();
        }


        let current_time = env::block_timestamp();
        assert!(
            current_time - self.last_compound_call >= 3_600_000_000_000, 
            "Compound can only be called once every hour"
        );

        self.last_compound_call = current_time;

        let mut promises = Vec::new();
    
        let owner = self.owner.get().as_ref().expect("Contract not initialized").clone();
        let main_account_id: AccountId = config::MAIN_ID
            .parse()
            .unwrap_or_else(|_| ContractError::InvalidAccountId(config::MAIN_ID.to_string()).panic());
    
        let get_user_promise = Promise::new(main_account_id)
            .function_call(
                "get_user".to_string(),
                json!({ "wallet_id": owner }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(20),
            )
            .then(
                Promise::new(env::current_account_id())
                    .function_call(
                        "compound_callback".to_string(),
                        json!({
                            "pre_claim_balance": U128(env::account_balance().as_yoctonear()),
                            "caller": env::predecessor_account_id(),
                        })
                        .to_string()
                        .into_bytes(),
                        NearToken::from_yoctonear(0),
                        Gas::from_tgas(250),
                    ),
            );
    
        promises.push(get_user_promise);
        promises
    }
    
    #[private]
    pub fn compound_callback(&mut self, pre_claim_balance: U128, caller: AccountId) -> PromiseOrValue<Vec<Promise>> {
        if env::promise_results_count() != 1 {
            env::log_str("Unexpected number of promise results from get_user");
            return PromiseOrValue::Value(vec![]);
        }
    
        let mut all_promises = Vec::new();
        let new_pref = Vec::new();
    
        match env::promise_result(0) {
            PromiseResult::Successful(result) => {
                let user_data: Value = serde_json::from_slice(&result)
                    .unwrap_or_else(|e| ContractError::SerializationFailed(e.to_string()).panic());
    
                env::log_str(&format!("Raw user data retrieved: {}", user_data));
    
                let preferences = user_data
                    .get("preferences")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| {
                        env::log_str("No preferences array found in user data");
                        &new_pref
                    });
    
                for pref in preferences {
                    env::log_str(&format!("Processing preference: {}", pref));
    
                    let seed_id = match pref.get("seed_id").and_then(|v| v.as_str()) {
                        Some(s) => String::from(s),
                        None => {
                            env::log_str("Skipping preference: Missing or invalid seed_id");
                            continue;
                        }
                    };
    
                    let token_id_str = match pref.get("token_id").and_then(|v| v.as_str()) {
                        Some(s) => String::from(s),
                        None => {
                            env::log_str("Skipping preference: Missing or invalid token_id");
                            continue;
                        }
                    };
                    let token_id: AccountId = match token_id_str.parse() {
                        Ok(id) => id,
                        Err(_) => {
                            env::log_str(&format!("Skipping preference: Invalid token_id {}", token_id_str));
                            continue;
                        }
                    };
    
                    let smart_contract_name_str = match pref.get("smart_contract_name").and_then(|v| v.as_str()) {
                        Some(s) => String::from(s),
                        None => {
                            env::log_str("Skipping preference: Missing or invalid smart_contract_name");
                            continue;
                        }
                    };
                    let smart_contract_name: AccountId = match smart_contract_name_str.parse() {
                        Ok(id) => id,
                        Err(_) => {
                            env::log_str(&format!("Skipping preference: Invalid smart_contract_name {}", smart_contract_name_str));
                            continue;
                        }
                    };
    

                    let is_active = pref
                        .get("is_active")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
    
                    // let reinvest_to = match pref.get("reinvest_to").and_then(|v| v.as_str()) {
                    //     Some(s) => String::from(s),
                    //     None => {
                    //         env::log_str("Skipping preference: Missing or invalid reinvest_to");
                    //         continue;
                    //     }
                    // };
    
                    if is_active {
                        let claim_all_rewards_promise = Promise::new(env::current_account_id())
                            .function_call(
                                "claim_all_rewards".to_string(),
                                json!({
                                    "seed_id": seed_id,
                                    "token_id": token_id,
                                })
                                .to_string()
                                .into_bytes(),
                                NearToken::from_yoctonear(0),
                                Gas::from_tgas(130),
                            );
    
                        let claim_from_burrow_promise = Promise::new(env::current_account_id())
                            .function_call(
                                "claim_from_burrow".to_string(),
                                json!({}).to_string().into_bytes(),
                                NearToken::from_yoctonear(0),
                                Gas::from_tgas(130),
                            );
    
                  
                        let combined_promise = claim_all_rewards_promise
                            .and(claim_from_burrow_promise)
                            .then(
                                Promise::new(env::current_account_id())
                                    .function_call(
                                        "handle_claim_result".to_string(),
                                        json!({
                                            "contract_name": smart_contract_name.to_string(),
                                            "action": "compound_and_reinvest"
                                        })
                                        .to_string()
                                        .into_bytes(),
                                        NearToken::from_yoctonear(0),
                                        Gas::from_tgas(10),
                                    ),
                            );
    
                        all_promises.push(combined_promise);
                    }
                }
    
                let post_claim_balance = env::account_balance().as_yoctonear();
                let balance_increase = post_claim_balance - pre_claim_balance.0;
    
                env::log_str(&format!(
                    "Pre-claim: {}, Post-claim: {}, Increase: {}",
                    pre_claim_balance.0, post_claim_balance, balance_increase
                ));
    
                if balance_increase > 2_000_000_000_000_000_000_000_000 {
                    let caller_share = balance_increase * 5 / 100;
                    env::log_str(&format!("Transferring {} yoctoNEAR to caller {}", caller_share, caller));
                    all_promises.push(
                        Promise::new(caller.clone()).transfer(NearToken::from_yoctonear(caller_share)),
                    );
                }
            }
            PromiseResult::Failed => {
                env::log_str("Failed to retrieve user data from get_user");
            }
        }
    
        PromiseOrValue::Value(all_promises)
    }



   

    


    pub fn reinvest(&mut self, contract_id: AccountId, min_amount_out: U128, pool_id: String, reinvest_to: String) -> Vec<Promise> {
        // self.assert_owner();
        let mut promises = Vec::new();

        let balance = env::account_balance().as_yoctonear();
        let two_near = 2_000_000_000_000_000_000_000_000;

        if balance <= two_near {
            env::log_str(&format!("Insufficient balance for reinvestment: {}", balance));
            return promises;
        }

        let stake_bal = balance - two_near;

        if reinvest_to == "Burrow" {
            let deposit_promise = Promise::new(contract_id.clone())
                .function_call(
                    "deposit_into_burrow".to_string(),
                    json!({
                        "deposit_amount": U128(stake_bal),
                    })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(200),
                )
                .then(
                    Promise::new(env::current_account_id())
                        .function_call(
                            "handle_reinvest_result".to_string(),
                            json!({
                                "contract_name": contract_id.to_string(),
                                "action": "deposit_into_burrow"
                            })
                            .to_string()
                            .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(10),
                        ),
                );
            promises.push(deposit_promise);
        } else if reinvest_to == "Stake" {
            let swap_promise = Promise::new(contract_id.clone())
                .function_call(
                    "stake_x_ref".to_string(),
                    json!({
                        "smart_contract_name": contract_id.to_string(),
                        "deposit_amount": U128(stake_bal),
                        "receiver_id": "xtoken.ref-finance.near",
                        "min_amount_out": min_amount_out,
                        "pool_id": pool_id,
                    })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(280),
                )
                .then(
                    Promise::new(env::current_account_id())
                        .function_call(
                            "handle_reinvest_result".to_string(),
                            json!({
                                "contract_name": contract_id.to_string(),
                                "action": "stake_x_ref"
                            })
                            .to_string()
                            .into_bytes(),
                            NearToken::from_yoctonear(0),
                            Gas::from_tgas(10),
                        ),
                );
            promises.push(swap_promise);
        } else {
            ContractError::InvalidInput(format!("Invalid reinvest_to option: {}", reinvest_to)).panic();
        }

        promises
    }

    #[private]
    pub fn handle_reinvest_result(&self, contract_name: String, action: String) -> PromiseOrValue<()> {
        if env::promise_results_count() > 0 {
            match env::promise_result(0) {
                PromiseResult::Successful(_) => {
                    env::log_str(&format!("{} succeeded for {}", action, contract_name));
                }
                PromiseResult::Failed => {
                    env::log_str(&format!(
                        "Failed to execute {} on {}: Check account registration or balance",
                        action, contract_name
                    ));
                }
            }
        } else {
            env::log_str("No promise result available");
        }
        PromiseOrValue::Value(())
    }

    pub fn stake_lp_tokens(&mut self, pool_id: String, lp_token_amount: U128, user_account: AccountId) -> Promise {
        self.assert_owner();
        
        if pool_id.is_empty() {
            ContractError::InvalidInput("pool_id cannot be empty".to_string()).panic();
        }
        if lp_token_amount.0 == 0 {
            ContractError::InvalidInput("lp_token_amount must be non-zero".to_string()).panic();
        }

        let storage_deposit_promise = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic())
        )
        .function_call(
            "storage_deposit".to_string(),
            json!({"account_id": user_account.to_string(), "registration_only": false})
                .to_string()
                .into_bytes(),
            NearToken::from_millinear(125),
            config::GAS_STORAGE_DEPOSIT,
        );

        let storage_deposit_promise2 = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic())
        )
        .function_call(
            "storage_deposit".to_string(),
            json!({"account_id": user_account.to_string(), "registration_only": false})
                .to_string()
                .into_bytes(),
            NearToken::from_millinear(100),
            config::GAS_STORAGE_DEPOSIT,
        );

        let transfer_promise = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic())
        )
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
            config::GAS_FT_TRANSFER,
        );

        storage_deposit_promise
            .then(storage_deposit_promise2)
            .then(transfer_promise)
    }

    pub fn unstake_lp(&mut self, seed_id: String, withdraw_amount: U128, token_id: AccountId) -> Promise {
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
        )
        .function_call(
            "unlock_and_withdraw_seed".to_string(),
            json!({
                "seed_id": seed_id,
                "unlock_amount": "0",
                "withdraw_amount": withdraw_amount,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER,
        );

        let withdraw_reward_token = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic())
        )
        .function_call(
            "withdraw_reward".to_string(),
            json!({"token_id": token_id})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD,
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
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic())
        )
        .function_call(
            "claim_reward_by_seed".to_string(),
            json!({"seed_id": seed_id, "token_id": token_id})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD,
        );

        let withdraw_lp_reward = Promise::new(
            config::BOOSTFARM
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BOOSTFARM.to_string()).panic())
        )
        .function_call(
            "withdraw_reward".to_string(),
            json!({"token_id": token_id})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD,
        );

        claim_reward.then(withdraw_lp_reward)
    }

    pub fn remove_liquidity_and_withdraw_tokens(&mut self, pool_id: u64, shares: U128, min_token_amount: U128, min_wrapped_near_amount: U128, token_id: AccountId, second_token_id: AccountId) -> Promise {
        self.assert_owner();
        if shares.0 == 0 {
            ContractError::InvalidInput("shares must be non-zero".to_string()).panic();
        }

        let remove_liquidity = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic())
        )
        .function_call(
            "remove_liquidity".to_string(),
            json!({
                "pool_id": pool_id,
                "shares": shares,
                "min_amounts": [min_token_amount, min_wrapped_near_amount],
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_REMOVE_LIQUIDITY,
        );

        let withdraw_wrap = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic())
        )
        .function_call(
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
            config::GAS_FT_TRANSFER,
        );

        let withdraw_black = Promise::new(
            config::REF_FINANCE
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::REF_FINANCE.to_string()).panic())
        )
        .function_call(
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
            config::GAS_FT_TRANSFER,
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
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BURROW.to_string()).panic())
        )
        .function_call(
            "storage_deposit".to_string(),
            json!({}).to_string().into_bytes(),
            NearToken::from_millinear(250),
            config::GAS_STORAGE_DEPOSIT,
        );

        let deposit_promise = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::WRAP.to_string()).panic())
        )
        .function_call(
            "near_deposit".to_string(),
            json!({}).to_string().into_bytes(),
            NearToken::from_yoctonear(deposit_amount.0),
            config::GAS_NEAR_DEPOSIT,
        );

        let collateral_transfer = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::WRAP.to_string()).panic())
        )
        .function_call(
            "ft_transfer_call".to_string(),
            json!({
                "receiver_id": config::BURROW,
                "amount": deposit_amount,
                "msg": json!({"Execute": {"actions": [{"IncreaseCollateral": {"token_id": config::WRAP, "max_amount": deposit_amount}}]}}).to_string()
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER,
        );

        storage_deposit_promise.then(deposit_promise).then(collateral_transfer)
    }

    pub fn deposit_into_burrow_pool(&mut self, token_id: AccountId, deposit_amount: U128) -> Promise {
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
            config::GAS_FT_TRANSFER,
        )
    }

    pub fn claim_from_burrow(&mut self) -> Promise {
        Promise::new(
            config::BURROW
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::BURROW.to_string()).panic())
        )
        .function_call(
            "account_farm_claim_all".to_string(),
            json!({}).to_string().into_bytes(),
            NearToken::from_yoctonear(0),
            config::GAS_CLAIM_REWARD,
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
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::ORACLE.to_string()).panic())
        )
        .function_call(
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
            config::GAS_ORACLE_CALL,
        );

        let wrap_promise2 = Promise::new(
            config::WRAP
                .parse()
                .unwrap_or_else(|_| ContractError::InvalidAccountId(config::WRAP.to_string()).panic())
        )
        .function_call(
            "near_withdraw".to_string(),
            json!({"amount": withdraw_amount})
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            config::GAS_FT_TRANSFER,
        );

        wrap_promise.then(wrap_promise2)
    }

    pub fn withdraw_token(&mut self, token_id: AccountId, receiver_id: AccountId, amount: U128) -> Promise {
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
            config::GAS_FT_TRANSFER,
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
            env::log_str(&format!(
                "Total balance: {}, Storage reserved: {}, Available: {}",
                total_balance, storage_cost, available_balance.as_yoctonear()
            ));
            ContractError::InsufficientBalance {
                available: available_balance.as_yoctonear(),
                requested: amount_near.as_yoctonear(),
            }
            .panic();
        }

        env::log_str(&format!("Initiating transfer of {} yoctoNEAR to beneficiary: {}", amount.0, beneficiary));
        Promise::new(beneficiary).transfer(amount_near)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{VMContextBuilder, accounts};
    use near_sdk::testing_env;

    #[test]
    fn test_new() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context);
        let contract = ProxyContract::new();
        assert_eq!(contract.get_owner(), accounts(0));
    }

    #[test]
    fn test_assert_owner_success() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context);
        let contract = ProxyContract::new();
        contract.assert_owner();
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_assert_owner_failure() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context);
        let contract = ProxyContract::new();
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(1))
            .build();
        testing_env!(context);
        contract.assert_owner();
    }

    #[test]
    #[should_panic(expected = "Invalid input: pool_id cannot be empty")]
    fn test_stake_lp_tokens_empty_pool_id() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_lp_tokens("".to_string(), U128(1_000), accounts(1));
    }

    #[test]
    #[should_panic(expected = "Invalid input: deposit_amount must be non-zero")]
    fn test_stake_x_ref_zero_deposit() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_x_ref(
            accounts(1),
            U128(0),
            accounts(2),
            U128(0),
            "123".to_string(),
        );
    }

    #[test]
    #[should_panic(expected = "Invalid input: pool_id cannot be empty")]
    fn test_stake_x_ref_empty_pool_id() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        contract.stake_x_ref(
            accounts(1),
            U128(1_000),
            accounts(2),
            U128(0),
            "".to_string(),
        );
    }



    #[test]
    fn test_reinvest_insufficient_balance() {
        let context = VMContextBuilder::new()
            .predecessor_account_id(accounts(0))
            .account_balance(NearToken::from_near(1))
            .build();
        testing_env!(context);
        let mut contract = ProxyContract::new();
        let promises = contract.reinvest(accounts(1), U128(0), "123".to_string(), "Burrow".to_string());
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
        let _promise = contract.withdraw_amount(accounts(1), U128(1_000_000_000_000_000_000_000_000));
    }
}