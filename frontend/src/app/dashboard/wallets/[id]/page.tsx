"use client";

import { use, useEffect, useState } from "react";
import Link from "next/link";
import { useAuth } from "@/lib/useAuth";
import {
  getWallet,
  getBalances,
  listAddresses,
  listTransactions,
  createAddress,
  stroopsToAmount,
  type WalletView,
  type Balance,
  type Address,
  type Transaction,
} from "@/lib/wallets";
import { WalletSidebar } from "@/components/dashboard/WalletSidebar";

export default function WalletOverview({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const { user, token, loading, logout } = useAuth();

  const [wallet, setWallet] = useState<WalletView | null>(null);
  const [balances, setBalances] = useState<Balance[]>([]);
  const [addresses, setAddresses] = useState<Address[]>([]);
  const [txns, setTxns] = useState<Transaction[]>([]);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    if (!token) return;
    getWallet(token, id).then(setWallet).catch(() => setWallet(null));
    getBalances(token, id).then(setBalances).catch(() => setBalances([]));
    listAddresses(token, id).then(setAddresses).catch(() => setAddresses([]));
    listTransactions(token, id).then(setTxns).catch(() => setTxns([]));
  }, [token, id]);

  async function onNewAddress() {
    if (!token) return;
    setCreating(true);
    try {
      const addr = await createAddress(token, id);
      setAddresses((a) => [addr, ...a]);
    } finally {
      setCreating(false);
    }
  }

  if (loading || !user) {
    return (
      <div className="flex min-h-screen items-center justify-center text-muted">
        Loading…
      </div>
    );
  }

  const xlm = balances.find((b) => b.asset_type === "native");
  const xlmAmount = xlm ? xlm.balance : "0";

  return (
    <div className="flex min-h-screen flex-col bg-background">
      <div className="bg-burgundy/20 py-2 text-center text-xs text-burgundy-bright">
        You are currently on <strong>test mode</strong> (Stellar testnet).
      </div>
      <div className="flex flex-1">
        <WalletSidebar
          walletId={id}
          walletName={wallet?.label ?? "Master wallet"}
        />

        <div className="flex flex-1 flex-col">
          {/* topbar */}
          <header className="flex items-center justify-between border-b border-white/10 px-8 py-4">
            <div className="flex items-center gap-2 text-sm text-muted">
              <Link href="/dashboard" className="hover:text-foreground">
                My Wallets
              </Link>
              <span>›</span>
              <span className="text-foreground">Overview</span>
            </div>
            <button
              onClick={logout}
              className="text-sm text-muted hover:text-foreground"
            >
              ⏻
            </button>
          </header>

          <main className="flex-1 space-y-6 px-8 py-8">
            {/* header */}
            <div>
              <h1 className="text-2xl font-semibold text-foreground">
                {wallet?.label ?? "Master wallet"}
              </h1>
              <p className="mt-1 text-sm text-muted">
                {wallet?.description ?? "Stellar master wallet"}
              </p>
              <div className="mt-3 flex flex-wrap gap-x-6 gap-y-1 text-xs">
                <span className="text-muted">
                  Address:{" "}
                  <span className="font-mono text-foreground">
                    {wallet
                      ? `${wallet.address.slice(0, 10)}…${wallet.address.slice(-8)}`
                      : "—"}
                  </span>
                </span>
                <span className="text-muted">
                  ID: <span className="font-mono text-foreground">{id.slice(0, 8)}…</span>
                </span>
              </div>
            </div>

            {/* stat cards */}
            <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
              <Stat label="Total Balance" value={`${xlmAmount} XLM`} />
              <Stat label="Current Balance" value={`${xlmAmount} XLM`} />
              <Stat label="Unswept Balance" value="0 XLM" sub="No sweep needed (muxed)" />
              <Stat label="No. of Assets" value={String(balances.length || 1)} />
            </div>

            {/* action row */}
            <div className="flex flex-wrap gap-3">
              <ActionButton label="New address" onClick={onNewAddress} loading={creating} />
              <ActionButton label="Deposit" disabled />
              <ActionButton label="Withdraw" disabled />
              <ActionButton label="Refresh balances" onClick={() => token && getBalances(token, id).then(setBalances)} />
            </div>

            <div className="grid gap-6 lg:grid-cols-[1.6fr_1fr]">
              {/* assets */}
              <Panel title="Assets">
                {balances.length === 0 ? (
                  <Empty>No assets yet.</Empty>
                ) : (
                  <ul className="divide-y divide-white/5">
                    {balances.map((b, i) => (
                      <li
                        key={i}
                        className="flex items-center justify-between py-3"
                      >
                        <span className="flex items-center gap-3">
                          <span className="flex h-8 w-8 items-center justify-center rounded-full bg-burgundy/30 text-xs text-burgundy-bright">
                            {b.asset_type === "native" ? "XLM" : b.asset_code ?? "?"}
                          </span>
                          <span className="text-sm text-foreground">
                            {b.asset_type === "native" ? "Stellar Lumens" : b.asset_code}
                          </span>
                        </span>
                        <span className="text-sm font-medium text-foreground">
                          {b.balance}{" "}
                          {b.asset_type === "native" ? "XLM" : b.asset_code}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </Panel>

              {/* addresses */}
              <Panel title="Addresses">
                {addresses.length === 0 ? (
                  <Empty>No addresses generated yet.</Empty>
                ) : (
                  <ul className="space-y-3">
                    {addresses.slice(0, 5).map((a) => (
                      <li key={a.id}>
                        <p className="font-mono text-xs text-burgundy-bright">
                          {a.muxed_address.slice(0, 8)}…{a.muxed_address.slice(-6)}
                        </p>
                        <p className="text-[11px] text-muted">
                          memo id {a.memo_id}
                          {a.customer_ref ? ` · ${a.customer_ref}` : ""}
                        </p>
                      </li>
                    ))}
                  </ul>
                )}
                <p className="mt-4 text-right text-xs text-muted">
                  Showing last {Math.min(addresses.length, 5)} generated
                </p>
              </Panel>
            </div>

            {/* recent transactions */}
            <Panel title="Most recent transactions">
              {txns.length === 0 ? (
                <Empty>No transactions yet.</Empty>
              ) : (
                <table className="w-full text-left text-sm">
                  <thead className="text-xs text-muted">
                    <tr>
                      <th className="py-2">ID</th>
                      <th className="py-2">Amount</th>
                      <th className="py-2">Hash</th>
                      <th className="py-2">Type</th>
                      <th className="py-2">Status</th>
                      <th className="py-2">Date</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-white/5">
                    {txns.map((t) => (
                      <tr key={t.id} className="text-foreground/90">
                        <td className="py-3 font-mono text-xs">
                          {t.id.slice(0, 8)}…
                        </td>
                        <td className="py-3">
                          {stroopsToAmount(t.amount_stroops)}{" "}
                          {t.asset_code === "native" ? "XLM" : t.asset_code}
                        </td>
                        <td className="py-3 font-mono text-xs">
                          {t.stellar_tx_hash
                            ? `${t.stellar_tx_hash.slice(0, 8)}…`
                            : "—"}
                        </td>
                        <td className="py-3">
                          <span className="rounded-md bg-white/5 px-2 py-0.5 text-xs capitalize">
                            {t.direction}
                          </span>
                        </td>
                        <td className="py-3">
                          <span className="text-xs text-burgundy-bright capitalize">
                            ● {t.status}
                          </span>
                        </td>
                        <td className="py-3 text-xs text-muted">
                          {new Date(t.created_at).toLocaleDateString()}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </Panel>
          </main>
        </div>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub?: string;
}) {
  return (
    <div className="rounded-xl border border-white/10 bg-burgundy-soft/30 p-4">
      <p className="text-[11px] text-muted">{label}</p>
      <p className="mt-1 text-xl font-semibold text-foreground">{value}</p>
      {sub && <p className="mt-1 text-[11px] text-muted">{sub}</p>}
    </div>
  );
}

function ActionButton({
  label,
  onClick,
  disabled,
  loading,
}: {
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  loading?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      className="rounded-lg border border-white/10 bg-white/[0.03] px-4 py-2 text-sm text-foreground transition-colors hover:border-burgundy/50 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {loading ? "…" : label}
    </button>
  );
}

function Panel({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-2xl border border-white/10 bg-burgundy-soft/20 p-5">
      <h3 className="text-sm font-semibold text-foreground">{title}</h3>
      <div className="mt-4">{children}</div>
    </section>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="py-6 text-center text-sm text-muted">{children}</p>;
}
