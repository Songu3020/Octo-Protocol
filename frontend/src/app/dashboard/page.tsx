"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useAuth } from "@/lib/useAuth";
import { listWallets, type WalletView } from "@/lib/wallets";
import { DashboardShell } from "@/components/dashboard/DashboardShell";

export default function DashboardHome() {
  const { user, token, loading, logout } = useAuth();
  const [wallets, setWallets] = useState<WalletView[] | null>(null);

  useEffect(() => {
    if (!token) return;
    listWallets(token)
      .then(setWallets)
      .catch(() => setWallets([]));
  }, [token]);

  if (loading || !user) {
    return (
      <div className="flex min-h-screen items-center justify-center text-muted">
        Loading…
      </div>
    );
  }

  const greeting = (() => {
    const h = new Date().getHours();
    return h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
  })();

  return (
    <DashboardShell email={user.email} title="Wallets" onLogout={logout}>
      <div className="mx-auto max-w-5xl">
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-3xl font-semibold text-foreground">
              {greeting},{" "}
              <span className="text-burgundy-bright">
                {user.email.split("@")[0]}
              </span>
            </h2>
            <p className="mt-1 text-sm text-muted">
              It&apos;s{" "}
              {new Date().toLocaleDateString("en-US", {
                weekday: "long",
                month: "short",
                day: "numeric",
                year: "numeric",
              })}
              .
            </p>
          </div>
          <Link
            href="/dashboard/wallets/new"
            className="rounded-full bg-burgundy px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-burgundy-bright"
          >
            New Master Wallet
          </Link>
        </div>

        <div className="my-8 h-px bg-white/10" />

        <h3 className="text-sm font-medium text-foreground">
          Your Master Wallets at a glance
        </h3>

        <div className="mt-5">
          {wallets === null ? (
            <p className="text-sm text-muted">Loading wallets…</p>
          ) : wallets.length === 0 ? (
            <EmptyState />
          ) : (
            <div className="grid gap-4 md:grid-cols-2">
              {wallets.map((w) => (
                <WalletCard key={w.id} wallet={w} />
              ))}
            </div>
          )}
        </div>
      </div>
    </DashboardShell>
  );
}

function EmptyState() {
  return (
    <div className="rounded-2xl border border-dashed border-white/15 bg-burgundy-soft/20 p-10 text-center">
      <p className="text-foreground">No master wallets yet</p>
      <p className="mt-1 text-sm text-muted">
        Create your first master wallet to start receiving deposits.
      </p>
      <Link
        href="/dashboard/wallets/new"
        className="mt-5 inline-block rounded-full bg-burgundy px-5 py-2.5 text-sm font-medium text-white hover:bg-burgundy-bright"
      >
        New Master Wallet
      </Link>
    </div>
  );
}

function WalletCard({ wallet }: { wallet: WalletView }) {
  const short = `${wallet.address.slice(0, 6)}…${wallet.address.slice(-6)}`;
  return (
    <div className="rounded-2xl border border-white/10 bg-burgundy-soft/30 p-5">
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-3">
          <span className="flex h-9 w-9 items-center justify-center rounded-full bg-burgundy/40 text-burgundy-bright">
            ◷
          </span>
          <div>
            <p className="font-semibold text-foreground">
              {wallet.label ?? "Master wallet"}
            </p>
            <p className="text-xs text-muted">
              {wallet.description ?? "Stellar master wallet"}
            </p>
          </div>
        </div>
        <ManageMenu walletId={wallet.id} />
      </div>

      <div className="mt-5 h-px bg-white/10" />

      <div className="mt-4 grid grid-cols-3 gap-3 text-xs">
        <div>
          <p className="text-muted">Network</p>
          <p className="mt-1 font-medium capitalize text-foreground">
            {wallet.network}
          </p>
        </div>
        <div>
          <p className="text-muted">Address</p>
          <p className="mt-1 font-mono text-foreground">{short}</p>
        </div>
        <div>
          <p className="text-muted">Base</p>
          <p className="mt-1 font-medium text-foreground">XLM</p>
        </div>
      </div>
    </div>
  );
}

function ManageMenu({ walletId }: { walletId: string }) {
  const [open, setOpen] = useState(false);

  return (
    <div className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1 rounded-lg border border-white/15 px-3 py-1.5 text-xs text-foreground hover:border-white/30"
      >
        Manage <span className="text-muted">⋮</span>
      </button>

      {open && (
        <>
          {/* click-away */}
          <div
            className="fixed inset-0 z-10"
            onClick={() => setOpen(false)}
          />
          <div className="absolute right-0 z-20 mt-2 w-44 overflow-hidden rounded-xl border border-white/10 bg-black/90 backdrop-blur-md">
            <Link
              href={`/dashboard/wallets/${walletId}`}
              className="flex items-center gap-2 px-4 py-3 text-sm text-foreground hover:bg-white/5"
            >
              ▦ Go to dashboard
            </Link>
            <Link
              href={`/dashboard/wallets/${walletId}/api`}
              className="flex items-center gap-2 px-4 py-3 text-sm text-foreground hover:bg-white/5"
            >
              ↗ API settings
            </Link>
          </div>
        </>
      )}
    </div>
  );
}
