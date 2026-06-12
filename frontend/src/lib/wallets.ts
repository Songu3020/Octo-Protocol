/** Wallet API calls + types, mirroring the octo backend. */

"use client";

import { apiFetch } from "./api";

export type CreateWalletResponse = {
  id: string;
  network: string;
  address: string;
  recovery_mnemonic: string;
  funded: boolean;
};

export type WalletView = {
  id: string;
  network: string;
  address: string;
  label: string | null;
  description: string | null;
};

export type Balance = {
  balance: string;
  asset_type: string;
  asset_code?: string | null;
  asset_issuer?: string | null;
};

export type Address = {
  id: string;
  customer_ref: string | null;
  muxed_address: string;
  base_address: string;
  memo_id: number;
  metadata: unknown;
};

export type Transaction = {
  id: string;
  direction: string;
  asset_code: string;
  amount_stroops: number;
  source_account: string | null;
  destination_account: string | null;
  stellar_tx_hash: string | null;
  status: string;
  created_at: string;
};

/** Create a master wallet. The server picks the network; name/description optional. */
export function createWallet(
  token: string,
  label?: string,
  description?: string,
) {
  return apiFetch<CreateWalletResponse>("/v1/wallets", {
    method: "POST",
    token,
    body: JSON.stringify({
      label: label || null,
      description: description || null,
    }),
  });
}

/** List the authenticated user's wallets. */
export function listWallets(token: string) {
  return apiFetch<WalletView[]>("/v1/wallets", { token });
}

export function getWallet(token: string, id: string) {
  return apiFetch<WalletView>(`/v1/wallets/${id}`, { token });
}

export function getBalances(token: string, id: string) {
  return apiFetch<Balance[]>(`/v1/wallets/${id}/balances`, { token });
}

export function listAddresses(token: string, id: string) {
  return apiFetch<Address[]>(`/v1/wallets/${id}/addresses`, { token });
}

export function createAddress(
  token: string,
  id: string,
  customerRef?: string,
) {
  return apiFetch<Address>(`/v1/wallets/${id}/addresses`, {
    method: "POST",
    token,
    body: JSON.stringify({ customer_ref: customerRef || null }),
  });
}

export function listTransactions(token: string, id: string) {
  return apiFetch<Transaction[]>(`/v1/wallets/${id}/transactions`, { token });
}

/** Format integer stroops as a decimal XLM-style string (7 dp). */
export function stroopsToAmount(stroops: number): string {
  return (stroops / 10_000_000).toFixed(7);
}

