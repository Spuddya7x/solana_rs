/**
 * Talking to the Exchange Clerk — the seam between the game and the chain.
 *
 * The clerk's dialogue is defined in
 * `server/content/scripts/bridge/scripts/exchange_clerk.rs2`:
 *
 * * **Talk-to** opens a four-way menu: withdraw GP, claim deposits, link a
 *   wallet, or nothing.
 * * **Using an item on her** withdraws that item, asking for a count.
 * * Withdrawals mint tokens to the linked wallet; deposits are burned on chain
 *   and then claimed here.
 *
 * Every path needs a linked wallet first, which is a one-time manual step: the
 * clerk shows a code, and `bun chain/cli/link-wallet.ts <code>` signs for it.
 */

import type { BotActions } from '../../../sdk/actions';
import type { BotSDK } from '../../../sdk/index';

/** How the clerk is matched in the world state. */
export const CLERK = /^exchange clerk$/i;

export interface BridgeResult {
    success: boolean;
    message: string;
}

/** Claim everything the chain side has deposited. */
export async function claimDeposits(
    bot: BotActions,
    sdk: BotSDK,
): Promise<BridgeResult & { claimed: number }> {
    const before = sdk.getInventory().length;
    const talk = await bot.talkTo(CLERK);
    if (!talk.success) {
        return { success: false, claimed: 0, message: `could not reach the clerk: ${talk.message}` };
    }
    // "Claim my deposits. (N waiting)" is the second option.
    await bot.navigateDialog([/claim my deposits/i]);
    await bot.waitForDialogClose(10_000);
    const after = sdk.getInventory().length;
    return {
        success: true,
        claimed: Math.max(after - before, 0),
        message: `inventory went from ${before} to ${after} occupied slots`,
    };
}

/**
 * Send GP to the linked wallet.
 *
 * The clerk asks for an amount through a count dialog, then a yes/no
 * confirmation. `amount` is clamped by the server to what is actually held.
 */
export async function withdrawGpToWallet(
    bot: BotActions,
    sdk: BotSDK,
    amount: number,
): Promise<BridgeResult> {
    if (amount <= 0) return { success: false, message: 'nothing to withdraw' };
    const talk = await bot.talkTo(CLERK);
    if (!talk.success) {
        return { success: false, message: `could not reach the clerk: ${talk.message}` };
    }
    await bot.navigateDialog([/withdraw gp to my wallet/i]);
    const entered = await sdk.sendCountDialog(amount);
    if (!entered.success) {
        return { success: false, message: `count dialog refused: ${entered.message}` };
    }
    // "Send N gp to my wallet." is the confirming option.
    await bot.navigateDialog([/send .* to my wallet/i]);
    await bot.waitForDialogClose(15_000);
    const left = sdk.countInventoryItems(/^coins$/i);
    return { success: true, message: `requested ${amount} gp, ${left} coins left in the inventory` };
}

/**
 * Send items to the linked wallet by using them on the clerk.
 *
 * This is the direction that turns a grind into tokens — and the exit route for
 * anything the on-chain side bought that turned out not to be worth alching.
 */
export async function withdrawItemToWallet(
    bot: BotActions,
    sdk: BotSDK,
    item: string | RegExp,
    amount: number,
): Promise<BridgeResult> {
    const held = sdk.findInventoryItem(item);
    if (!held) return { success: false, message: `no ${item} in the inventory` };
    const used = await bot.useItemOnNpc(held, CLERK);
    if (!used.success) {
        return { success: false, message: `could not use ${held.name} on the clerk: ${used.message}` };
    }
    const entered = await sdk.sendCountDialog(amount);
    if (!entered.success) {
        return { success: false, message: `count dialog refused: ${entered.message}` };
    }
    await bot.navigateDialog([/send .* to my wallet/i]);
    await bot.waitForDialogClose(15_000);
    return { success: true, message: `requested ${amount} x ${held.name}` };
}

/**
 * Show the one-time wallet link code.
 *
 * Linking cannot be automated from here: the code has to be signed by the wallet
 * with `bun chain/cli/link-wallet.ts <code>`. This surfaces the dialogue so a
 * human (or an outer script reading chat) can pick the code up.
 */
export async function requestLinkCode(bot: BotActions, sdk: BotSDK): Promise<BridgeResult> {
    const talk = await bot.talkTo(CLERK);
    if (!talk.success) {
        return { success: false, message: `could not reach the clerk: ${talk.message}` };
    }
    await bot.navigateDialog([/link my solana wallet/i]);
    const message = await sdk.waitForChat({ matching: /link code/i, timeout: 10_000 });
    return {
        success: true,
        message: message?.text ?? 'link code shown in the game window — read it from the dialogue',
    };
}
