#  Ref Finance auto-claim  smart contract

This is a NEAR Protocol smart contract that enables users stake, auto claim and compound rewards. The contract is written in Rust and deployed on the NEAR blockchain.


## Features
- Create and manage Subaccounts 

- Stake Tokens: Users can stake tokens to earn rewards.

- Claim Rewards: Users can claim their staking rewards.

- View Data: Users can query their staked balance and rewards.

- Withdraw rewards



## Technologies Used
- Rust: The programming language used to write the smart contract.

- NEAR SDK: For interacting with the NEAR blockchain.

- NEAR Mainnet: The contract is deployed on the NEAR Mainnet for testing.


## Contract ID
```
[userid].auto-claim-main2.near
```

# Contract ID

### lib.rs
This contract is deployed into sub-accounts created by users. It enables users to:

- Stake tokens: Users can stake their tokens in supported pools.

- Deposit into Burrow: Users can deposit tokens into Burrow for lending/borrowing.

- Manage tokens: Users can manage their token balances and transactions efficiently.








## How to Build Locally?

Install [`cargo-near`](https://github.com/near/cargo-near) and run:

```bash
cargo near build
```

## How to Test Locally?

```bash
cargo test
```


## How to Deploy?

Deployment is automated with GitHub Actions CI/CD pipeline.
To deploy manually, install [`cargo-near`](https://github.com/near/cargo-near) and run:

```bash
cargo near deploy build-reproducible-wasm <account-id>
```

## Useful Links

- [cargo-near](https://github.com/near/cargo-near) - NEAR smart contract development toolkit for Rust
- [near CLI](https://near.cli.rs) - Interact with NEAR blockchain from command line
- [NEAR Rust SDK Documentation](https://docs.near.org/sdk/rust/introduction)
- [NEAR Documentation](https://docs.near.org)
- [NEAR StackOverflow](https://stackoverflow.com/questions/tagged/nearprotocol)
- [NEAR Discord](https://near.chat)
- [NEAR Telegram Developers Community Group](https://t.me/neardev)
- NEAR DevHub: [Telegram](https://t.me/neardevhub), [Twitter](https://twitter.com/neardevhub)
