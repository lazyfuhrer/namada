//! A tx for adding a transfer request across the Ethereum bridge
//! into the bridge pool.
use borsh::{BorshDeserialize, BorshSerialize};
use eth_bridge::storage::{bridge_pool, native_erc20_key};
use eth_bridge_pool::{GasFee, PendingTransfer, TransferToEthereum};
use namada_tx_prelude::*;

#[transaction(gas = 100000)]
fn apply_tx(ctx: &mut Ctx, signed: Tx) -> TxResult {
    let data = signed.data().ok_or_err_msg("Missing data")?;
    let transfer = PendingTransfer::try_from_slice(&data[..])
        .map_err(|e| Error::wrap("Error deserializing PendingTransfer", e))?;
    log_string("Received transfer to add to pool.");
    // pay the gas fees
    let GasFee {
        token: ref fee_token_addr,
        amount,
        ref payer,
    } = transfer.gas_fee;
    token::transfer(
        ctx,
        payer,
        &bridge_pool::BRIDGE_POOL_ADDRESS,
        fee_token_addr,
        amount.native_denominated(),
        &None,
        &None,
        &None,
    )?;
    log_string("Token transfer succeeded.");
    let TransferToEthereum {
        asset,
        ref sender,
        amount,
        ..
    } = transfer.transfer;
    // if minting wNam, escrow the correct amount
    if asset == native_erc20_address(ctx)? {
        let nam_addr = ctx.get_native_token()?;
        token::transfer(
            ctx,
            sender,
            &eth_bridge::ADDRESS,
            &nam_addr,
            amount.native_denominated(),
            &None,
            &None,
            &None,
        )?;
    } else {
        // Otherwise we escrow ERC20 tokens.
        let token = transfer.token_address();
        token::transfer(
            ctx,
            sender,
            &bridge_pool::BRIDGE_POOL_ADDRESS,
            &token,
            amount.native_denominated(),
            &None,
            &None,
            &None,
        )?;
    }
    log_string("Escrow succeeded");
    // add transfer into the pool
    let pending_key = bridge_pool::get_pending_key(&transfer);
    ctx.write_bytes(&pending_key, transfer.try_to_vec().unwrap())
        .wrap_err("Could not write transfer to bridge pool")?;
    Ok(())
}

fn native_erc20_address(ctx: &mut Ctx) -> EnvResult<EthAddress> {
    log_string("Trying to get wnam key");
    let addr = ctx
        .read_bytes(&native_erc20_key())
        .map_err(|e| Error::wrap("Could not read wNam key from storage", e))?
        .unwrap();
    log_string("Got wnam key");
    Ok(BorshDeserialize::try_from_slice(addr.as_slice()).unwrap())
}
