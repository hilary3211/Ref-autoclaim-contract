use near_sdk::{near,env, Gas, NearToken, Promise, serde_json, AccountId, PromiseResult,  PromiseOrValue};
use serde_json::Value;
use serde_json::json;
#[near(contract_state)]
#[derive(Default)]
pub struct ProxyContract {}



#[near]
impl ProxyContract {

    fn assert_only_owner(&self) {
       
         let contract_id = env::current_account_id().to_string();


         let name = contract_id
             .split('.')
             .next()
             .expect("Invalid contract ID format");
 
       
         let owner_id = format!("{}.near", name);
 
         // Get the signer's account ID
         let signer_id = env::signer_account_id().to_string();
         let main_id = "auto-claim-main.near".to_string();
 
    
         assert!(
             signer_id == owner_id || signer_id == contract_id || signer_id == main_id,
             "Only {} or {} can interact with this contract",
             owner_id,
             contract_id
         );
    }


    pub fn add_liq(
        &self,
        tokenamount: String,
        wrappednearamount: String,
        poolid: String,
        tokenname: String,
        userid: String,
        gassing: String,
        adddepo : String
    ) {
        self.assert_only_owner();
        let neardeposit: u128 = wrappednearamount.parse().expect("Invalid deposit value");
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
        let poolid2: u64 = poolid.parse().expect("Invalid pool ID");
        let depo: u128 = adddepo.parse().expect("Invalid deposit value");
        let deposit_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
            "near_deposit".to_string(),
            "{}".as_bytes().to_vec(),
            NearToken::from_yoctonear(neardeposit),
            Gas::from_tgas(neargas),
        );
    
        // Convert deposited NEAR to WNEAR
        let wrap_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
            "near_withdraw".to_string(),
            json!({"amount": wrappednearamount}).to_string().into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(neargas),
        );
    
        // Deposit storage for user on ref-finance
        let storage_deposit_args = json!({ "account_id": userid, "registration_only": false })
            .to_string()
            .into_bytes();
        let token_deposit_promise = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
            "storage_deposit".to_string(),
            storage_deposit_args,
            NearToken::from_yoctonear(1250000000000000000000),
            Gas::from_tgas(neargas),
        );
    
        // Transfer wrapped NEAR (after wrapping) to ref-finance
        let wrap_transfer_args = json!({
            "receiver_id": "v2.ref-finance.near",
            "amount": wrappednearamount,
            "msg": ""
        })
        .to_string()
        .into_bytes();
        let wrap_transfer_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
            "ft_transfer_call".to_string(),
            wrap_transfer_args,
            NearToken::from_yoctonear(1),
            Gas::from_tgas(neargas),
        );
    
        // Transfer token to ref-finance
        let token_transfer_args = json!({
            "receiver_id": "v2.ref-finance.near",
            "amount": tokenamount,
            "msg": ""
        })
        .to_string()
        .into_bytes();
        let token_transfer_promise = Promise::new(tokenname.parse().unwrap()).function_call(
            "ft_transfer_call".to_string(),
            token_transfer_args,
            NearToken::from_yoctonear(1),
            Gas::from_tgas(neargas),
        );
    
        // Add liquidity to the pool
        let add_liquidity_args = json!({
            "pool_id": poolid2,
            "amounts": [tokenamount, wrappednearamount]
        })
        .to_string()
        .into_bytes();
        let add_liq_promise = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
            "add_liquidity".to_string(),
            add_liquidity_args,
            NearToken::from_yoctonear(depo),
            Gas::from_tgas(neargas),
        );
    
        // Chain promises in correct order
        deposit_promise
            .then(wrap_promise)
            .then(token_deposit_promise)
            .then(wrap_transfer_promise)
            .then(token_transfer_promise)
            .then(add_liq_promise);
    }





    














    // pub fn add_liq(
    //     &self,
    //     tokenamount: String,
    //     wrappednearamount: String,
    //     poolid: String,
    //     tokenname: String,
    //     userid: String,
    //     gassing: String
    // ) {
    //     self.assert_only_owner();
    //     let neardeposit: u128 = wrappednearamount.parse().expect("Invalid deposit value");
    //     let neargas: u64 = gassing.parse().expect("Invalid gas value");
    //     let poolid2: u64 = poolid.parse().expect("Invalid pool ID");
    
    //     // Deposit NEAR into wrap.near
    //     let deposit_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
    //         "near_deposit".to_string(),
    //         "{}".as_bytes().to_vec(),
    //         NearToken::from_yoctonear(neardeposit),
    //         Gas::from_tgas(neargas),
    //     );
    
    //     // Convert deposited NEAR to WNEAR
    //     let wrap_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
    //         "near_withdraw".to_string(),
    //         json!({"amount": wrappednearamount}).to_string().into_bytes(),
    //         NearToken::from_yoctonear(1),
    //         Gas::from_tgas(neargas),
    //     );
    
    //     // Deposit storage for user on ref-finance
    //     let storage_deposit_args = json!({ "account_id": userid, "registration_only": false })
    //         .to_string()
    //         .into_bytes();
    //     let token_deposit_promise = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
    //         "storage_deposit".to_string(),
    //         storage_deposit_args,
    //         NearToken::from_yoctonear(2500000000000000000000), // 0.00125 NEAR
    //         Gas::from_tgas(neargas),
    //     );
    
    //     // Transfer wrapped NEAR (after wrapping) to ref-finance
    //     let wrap_transfer_args = json!({
    //         "receiver_id": "v2.ref-finance.near",
    //         "amount": wrappednearamount,
    //         "msg": ""
    //     })
    //     .to_string()
    //     .into_bytes();
    //     let wrap_transfer_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
    //         "ft_transfer_call".to_string(),
    //         wrap_transfer_args,
    //         NearToken::from_yoctonear(1),
    //         Gas::from_tgas(neargas),
    //     );
    
    //     // Transfer token to ref-finance
    //     let token_transfer_args = json!({
    //         "receiver_id": "v2.ref-finance.near",
    //         "amount": tokenamount,
    //         "msg": ""
    //     })
    //     .to_string()
    //     .into_bytes();
    //     let token_transfer_promise = Promise::new(tokenname.parse().unwrap()).function_call(
    //         "ft_transfer_call".to_string(),
    //         token_transfer_args,
    //         NearToken::from_yoctonear(1),
    //         Gas::from_tgas(neargas),
    //     );
    
    //     // Add liquidity to the pool with retry logic
    //     let add_liquidity_args = json!({
    //         "pool_id": poolid2,
    //         "amounts": [tokenamount, wrappednearamount]
    //     })
    //     .to_string()
    //     .into_bytes();
    
    //     // Initial deposit amount
    //     let mut deposit_amount = 900000000000000000000; // 0.00092 NEAR
    //     let max_deposit = 1000000000000000000000; // 0.001 NEAR
    //     let increment = 10000000000000000000; // 0.00001 NEAR
    
    //     // Retry loop
    //     while deposit_amount <= max_deposit {
    //         let add_liq_promise = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
    //             "add_liquidity".to_string(),
    //             add_liquidity_args.clone(),
    //             NearToken::from_yoctonear(deposit_amount),
    //             Gas::from_tgas(neargas),
    //         );
    
    //         // Execute the promise and check for errors
    //         match add_liq_promise.then(Self::ext(env::current_account_id()).check_result()) {
    //             Ok(_) => {
    //                 // Success: break the loop
    //                 break;
    //             }
    //             Err(err) => {
    //                 // Check if the error is due to insufficient storage deposit
    //                 if err.to_string().contains("ERR_STORAGE_DEPOSIT") {
    //                     // Increment the deposit amount and retry
    //                     deposit_amount += increment;
    //                 } else {
    //                     // Propagate other errors
    //                     panic!("Error adding liquidity: {}", err);
    //                 }
    //             }
    //         }
    //     }
    
    //     // Chain promises in correct order
    //     deposit_promise
    //         .then(wrap_promise)
    //         .then(token_deposit_promise)
    //         .then(wrap_transfer_promise)
    //         .then(token_transfer_promise);
    // }
   
    

  

    

    pub fn stake_lp_tokens(&self, pool_id: String, lp_token_amount: String, gassing: String,useracc : String ) -> Promise {
   

        self.assert_only_owner();

        let boostfarm = "boostfarm.ref-labs.near".to_string();
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
    
        // Step 1: Register the boostfarm account with v2.ref-finance.near
        let storage_deposit_promise = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
            "storage_deposit".parse().unwrap(),
            json!({
                "account_id": useracc,
                "registration_only": false // Allow full usage of storage
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1250000000000000000000), // 1.25 NEAR for storage deposit
            Gas::from_tgas(neargas),
        );

        let storage_deposit_promise2 = Promise::new("boostfarm.ref-labs.near".parse().unwrap()).function_call(
            "storage_deposit".parse().unwrap(),
            json!({
                "account_id": useracc,
                "registration_only": false // Allow full usage of storage
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(100000000000000000000000), // 1.25 NEAR for storage deposit
            Gas::from_tgas(neargas),
        );
    
        // Step 2: Transfer LP tokens to boostfarm
        let transfer_promise = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
            "mft_transfer_call".to_string(),
            json!({
                "receiver_id": boostfarm,
                "token_id": pool_id,
                "amount": lp_token_amount,
                "msg": "\"Free\""
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1), // Attach 1 yoctoNEAR
            Gas::from_tgas(neargas),
        );
    
        // Chain the promises
        storage_deposit_promise.then(storage_deposit_promise2).then(transfer_promise)
    }


    pub fn unstake_lp(&self, seed_id: String, withdraw_amount: String, gassing: String, tokenname : String) -> Promise {
        self.assert_only_owner();
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
        let unlock_and_withdraw_seed = Promise::new("boostfarm.ref-labs.near".parse().unwrap()).function_call(
            "unlock_and_withdraw_seed".to_string(),
            json!({
                "seed_id": seed_id,//"v2.ref-finance.near@79",
                "unlock_amount": "0",
                "withdraw_amount": withdraw_amount,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1),
           Gas::from_tgas(neargas),
        );

        let withdraw_reward_token = Promise::new("boostfarm.ref-labs.near".parse().unwrap()).function_call(
            "withdraw_reward".to_string(),
            json!({ "token_id": tokenname })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(neargas),
        );



        unlock_and_withdraw_seed
            .then(withdraw_reward_token)

    }


    pub fn claim_all_rewards(&self, seed_id: String, gassing: String, tokenid : String) -> Promise {
        self.assert_only_owner();
        let neargas: u64 = gassing.parse().expect("Invalid gas value");


        let claim_reward = Promise::new("boostfarm.ref-labs.near".parse().unwrap())

            .function_call(
                "claim_reward_by_seed".to_string(),
                json!({ "seed_id": seed_id }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(neargas),
            );

        let withdraw_lp_reward = Promise::new("boostfarm.ref-labs.near".parse().unwrap())
            .function_call(
                "withdraw_reward".to_string(),
                json!({ "token_id": tokenid }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(neargas),
            );


        claim_reward.then(withdraw_lp_reward)
 
    }

    pub fn remove_liquidity_and_withdraw_tokens(
        &self,
        pool_id: u64,
        shares: String,
        tokenamount: String,
        wrappednearamount: String,
        tokenname: String,
        tokenname2: String,
        gassing : String
    ) -> Promise {

        self.assert_only_owner();
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
        let remove_liquidity = Promise::new("v2.ref-finance.near".parse().unwrap())
            .function_call(
                "remove_liquidity".to_string(),
                near_sdk::serde_json::json!({
                    "pool_id": pool_id,
                    "shares": shares,
                    "min_amounts": [tokenamount, wrappednearamount],
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(neargas),

            );
        
        let withdraw_wrap = Promise::new("v2.ref-finance.near".parse().unwrap())
            .function_call(
                "withdraw".to_string(),
                near_sdk::serde_json::json!({
                    "token_id": tokenname,
                    "amount": "0",
                    "unregister": false,
                    "skip_unwrap_near": false,
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(neargas),
            );
        
        let withdraw_black = Promise::new("v2.ref-finance.near".parse().unwrap())
            .function_call(
                "withdraw".to_string(),
                near_sdk::serde_json::json!({
                    "token_id": tokenname2,
                    "amount": "0",
                    "unregister": false,
                    "skip_unwrap_near": false,
                })
                .to_string()
                .into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(neargas),
            );
        
        remove_liquidity.then(withdraw_wrap).then(withdraw_black)
    }




    pub fn deposit_into_burrow(&self, deposit_amount: String, gassing: String) -> Promise {
        self.assert_only_owner();
        let neardeposit: u128 = deposit_amount.parse().expect("Invalid deposit value");
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
    
        // Step 1: Register the account with contract.main.burrow.near (if not already registered)
        let storage_deposit_promise = Promise::new("contract.main.burrow.near".parse().unwrap()).function_call(
            "storage_deposit".to_string(),
            "{}".to_string().into_bytes(),
            NearToken::from_yoctonear(250000000000000000000000), // 1.25 NEAR for storage deposit
            Gas::from_tgas(neargas),
        );
    
        // Step 2: Deposit NEAR into wrap.near to get wNEAR
        let deposit_promise = Promise::new("wrap.near".parse().unwrap()).function_call(
            "near_deposit".to_string(),
            "{}".to_string().into_bytes(),
            NearToken::from_yoctonear(neardeposit), // Amount of NEAR to deposit
            Gas::from_tgas(neargas),
        );
    
        // Step 3: Transfer wNEAR to contract.main.burrow.near to increase collateral
        let collateral_transfer = Promise::new("wrap.near".parse().unwrap()).function_call(
            "ft_transfer_call".to_string(),
            json!({
                "receiver_id": "contract.main.burrow.near",
                "amount": deposit_amount,
                "msg": json!({
                    "Execute": {
                        "actions": [{
                            "IncreaseCollateral": {
                                "token_id": "wrap.near",
                                "max_amount": deposit_amount
                            }
                        }]
                    }
                }).to_string()
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1), 
            Gas::from_tgas(neargas),
        );
    

        storage_deposit_promise
            .then(deposit_promise)
            .then(collateral_transfer)
    }

    pub fn withdraw_from_borrow_pool(&self, withdraw_amount: String, gassing : String) -> Promise {
        self.assert_only_owner();
        
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
        let payload = near_sdk::serde_json::json!({
            "receiver_id": "contract.main.burrow.near",
            "msg": near_sdk::serde_json::json!({
                "Execute": {
                    "actions": [
                        { "DecreaseCollateral": { "token_id": "wrap.near",} },
                        { "Withdraw": { "token_id": "wrap.near" } }
                    ]
                }
            }).to_string()
        });
     
        let wrap_promise = Promise::new("priceoracle.near".parse().unwrap())
            .function_call(
                "oracle_call".to_string(),
                payload.to_string().into_bytes(),
                NearToken::from_yoctonear(1),
            Gas::from_tgas(neargas),
            );

    let wrap_promise2 = Promise::new("wrap.near".parse().unwrap()).function_call(
                "near_withdraw".to_string(),
                json!({"amount": withdraw_amount}).to_string().into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(neargas),
            );

            wrap_promise.then(wrap_promise2)
    }



    pub fn stake_xRef(&self, smart_contract_name: String, transfer_call_args: String, deposit_amount : String , gassing : String, receiver_id : String, min_amount_out : String, pool_id: String ) {
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
        let neardeposit: u128 = deposit_amount.parse().expect("Invalid deposit value");

        let transfer_args = json!({
            "receiver_id" : receiver_id,
            "amount" : deposit_amount,
            "msg" : json!({
                "force": 0,
                "actions": [
                    {
                        "pool_id": pool_id,
                        "token_in": "wrap.near",
                        "token_out": "xtoken.ref-finance.near",
                        "amount_in": deposit_amount,
                        "min_amount_out": min_amount_out,
                    }
                ]
            })
            .to_string()
        })
        .to_string()
        .into_bytes();

       
        let p0 = Promise::new("wrap.near".parse().unwrap()).function_call(
            "near_deposit".to_string(),
            "{}".as_bytes().to_vec(),
            NearToken::from_yoctonear(neardeposit),
            Gas::from_tgas(neargas),
        );


        let p1 = Promise::new("wrap.near".parse().unwrap()).function_call(
            "storage_deposit".to_string(),
            json!({
                "account_id": smart_contract_name,
                "registration_only": true,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1250000000000000000000),
            Gas::from_tgas(neargas),
        );

      
        let p2 = Promise::new("token.v2.ref-finance.near".parse().unwrap()).function_call(
            "storage_deposit".to_string(),
            json!({
                "account_id": smart_contract_name,
                "registration_only": true,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1250000000000000000000),
            Gas::from_tgas(neargas),
        );

        // Third storage_deposit call to xtoken.ref-finance.near
        let p3 = Promise::new("xtoken.ref-finance.near".parse().unwrap()).function_call(
            "storage_deposit".to_string(),
            json!({
                "account_id": smart_contract_name,
                "registration_only": true,
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1250000000000000000000),
            Gas::from_tgas(neargas),
        );


        let p4 = Promise::new("v2.ref-finance.near".parse().unwrap()).function_call(
            "ft_transfer_call".to_string(),
            transfer_args,
            NearToken::from_yoctonear(1),
            Gas::from_tgas(neargas),
        );


        p0.and(p1).and(p2).and(p3).then(p4);
    }



    


    pub fn withdraw_token(
        &self,
        token_id: String,
        receiver_id: String,
        amount: String,
        gassing: String,
    ) -> Promise {
        let neargas: u64 = gassing.parse().expect("Invalid gas value");
        self.assert_only_owner();
        // Transfer the token to the receiver
        Promise::new(token_id.parse().unwrap()).function_call(
            "ft_transfer".to_string(),
            json!({
                "receiver_id": receiver_id,
                "amount": amount,
                "memo": "Withdraw token from contract"
            })
            .to_string()
            .into_bytes(),
            NearToken::from_yoctonear(1), 
            Gas::from_tgas(neargas),
        )
    }

    pub fn get_contract_balance(&self) -> NearToken {
        self.assert_only_owner();
        env::account_balance()
    }


    #[payable]
    pub fn withdraw_all_and_delete(&mut self, beneficiary: AccountId) {
        // Ensure only the owner can call this function.
        self.assert_only_owner();

        // Get the current balance of the contract.
        let current_balance = env::account_balance();

        // Transfer the entire balance to the beneficiary.
        Promise::new(beneficiary.clone()).transfer(current_balance);

        // Create a promise batch with a reference to the beneficiary.
        let batch_id = env::promise_batch_create(&beneficiary);

        // Delete the contract and send any remaining funds to the beneficiary.
        env::promise_batch_action_delete_account(batch_id, &beneficiary);
    }
    
    
    

}






