use aleph_client::{
    account_from_keypair,
    api::treasury::events::Rejected,
    pallets::{
        balances::{BalanceApi, BalanceUserApi},
        staking::StakingApi,
        system::SystemApi,
        treasury::{TreasuryApi, TreasuryUserApi},
    },
    utility::BlocksApi,
    waiting::{AlephWaiting, BlockStatus},
    AsConnection, ConnectionApi, KeyPair, RootConnection, SignedConnection, TxStatus,
};
use log::info;
use primitives::Balance;

use crate::{accounts::get_validators_raw_keys, config::setup_test, transfer::setup_for_transfer};

/// Returns current treasury free funds and total issuance.
///
/// Takes two storage reads.
async fn balance_info<C: ConnectionApi + AsConnection>(connection: &C) -> (Balance, Balance) {
    let treasury_balance = connection
        .get_free_balance(connection.treasury_account().await, None)
        .await;
    let issuance = connection.total_issuance(None).await;
    info!(
        "[+] Treasury balance: {}. Total issuance: {}.",
        treasury_balance, issuance
    );

    (treasury_balance, issuance)
}

#[tokio::test]
pub async fn channeling_fee_and_tip() -> anyhow::Result<()> {
    let config = setup_test();
    let (transfer_amount, tip) = (1_000u128, 10_000u128);
    let (connection, to) = setup_for_transfer(config).await;

    // We need to make sure the rest of this test happens during a single era,
    // so that when we check the total issuance we don't have to take into account
    // the tokens minted when an era ends.
    // Therefore, let's wait for when the next era begins, and keep our fingers crossed.
    let active_era = connection.get_active_era(None).await;
    connection
        .wait_for_era(active_era + 1, BlockStatus::Best)
        .await;

    let (treasury_balance_before, issuance_before) = balance_info(&connection).await;

    let transfer = connection
        .transfer_keep_alive_with_tip(to, transfer_amount, tip, TxStatus::Finalized)
        .await?;
    let fee = connection.get_tx_fee(transfer).await?;

    let (treasury_balance_after, issuance_after) = balance_info(&connection).await;

    check_issuance(issuance_before, issuance_after);
    check_treasury_balance(treasury_balance_before, treasury_balance_after, fee);

    Ok(())
}

fn check_issuance(issuance_before: Balance, issuance_after: Balance) {
    assert!(
        issuance_after >= issuance_before,
        "Unexpectedly {} was burned",
        issuance_before - issuance_after,
    );
    let minted = issuance_after - issuance_before;
    assert_eq!(
        issuance_before, issuance_after,
        "Unexpectedly {minted} was minted"
    );
}

fn check_treasury_balance(
    treasury_balance_before: Balance,
    treasury_balance_after: Balance,
    fee: Balance,
) {
    let treasury_balance_diff = treasury_balance_after - (treasury_balance_before + fee);
    assert_eq!(
        treasury_balance_diff, 0,
        "Incorrect amount was channeled to the treasury: before = {}, after = {}, fee = {}",
        treasury_balance_before, treasury_balance_after, fee,
    );
}

#[tokio::test]
pub async fn treasury_access() -> anyhow::Result<()> {
    let config = setup_test();
    let proposer = KeyPair::new(get_validators_raw_keys(config)[0].clone());
    let beneficiary = account_from_keypair(proposer.signer());
    let connection = SignedConnection::new(&config.node, proposer).await;

    let proposals_counter_before = connection.proposals_count(None).await.unwrap_or_default();
    connection
        .propose_spend(10, beneficiary.clone(), TxStatus::InBlock)
        .await?;
    connection
        .propose_spend(100, beneficiary.clone(), TxStatus::InBlock)
        .await?;
    let proposals_counter_after = connection.proposals_count(None).await.unwrap_or_default();

    assert_eq!(
        proposals_counter_before + 2,
        proposals_counter_after,
        "Proposal has not been created"
    );

    let root_connection = config.create_root_connection().await;

    approve_treasury_proposal(&root_connection, proposals_counter_after - 2).await?;
    reject_treasury_proposal(&root_connection, proposals_counter_after - 1).await?;

    Ok(())
}

async fn approve_treasury_proposal(connection: &RootConnection, id: u32) -> anyhow::Result<()> {
    connection.approve(id, TxStatus::Finalized).await?;
    let approvals = connection.approvals(None).await;
    assert!(approvals.contains(&id));

    Ok(())
}

async fn reject_treasury_proposal(connection: &RootConnection, id: u32) -> anyhow::Result<()> {
    let handle_connection = connection.clone();
    let handle = tokio::spawn(async move {
        handle_connection
            .wait_for_event(
                |e: &Rejected| e.proposal_index == id,
                BlockStatus::Finalized,
            )
            .await;
    });
    connection.reject(id, TxStatus::Finalized).await?;
    handle.await?;

    Ok(())
}
