"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { Logo } from "@/components/Logo";

export function WalletSidebar({
  walletId,
  walletName,
}: {
  walletId: string;
  walletName: string;
}) {
  const pathname = usePathname();
  const base = `/dashboard/wallets/${walletId}`;

  const NAV = [
    { label: "Overview", href: base, icon: "▦" },
    { label: "Assets", href: `${base}/assets`, icon: "$" },
    { label: "Transactions", href: `${base}/transactions`, icon: "◷" },
    { label: "Addresses", href: `${base}/addresses`, icon: "▢" },
    { label: "Beneficiaries", href: `${base}/beneficiaries`, icon: "⚇" },
  ];

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-white/10 bg-black/40 px-3 py-5">
      <div className="px-2">
        <Logo />
      </div>

      {/* wallet selector chip */}
      <div className="mt-5 rounded-xl border border-white/10 bg-white/[0.03] px-3 py-2.5">
        <p className="truncate text-sm font-semibold text-foreground">
          {walletName}
        </p>
        <div className="mt-1 flex gap-1.5">
          <span className="rounded-md bg-burgundy/30 px-2 py-0.5 text-[10px] text-burgundy-bright">
            Stellar
          </span>
          <span className="rounded-md bg-white/5 px-2 py-0.5 text-[10px] text-muted">
            Testnet
          </span>
        </div>
      </div>

      <nav className="mt-6 flex-1 space-y-1">
        <Link
          href="/dashboard"
          className="flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-foreground"
        >
          <span className="w-4 text-center opacity-80">⌂</span> My Wallets
        </Link>
        {NAV.map((item) => {
          const active =
            item.href === base ? pathname === base : pathname.startsWith(item.href);
          return (
            <Link
              key={item.label}
              href={item.href}
              className={`flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm transition-colors ${
                active
                  ? "bg-burgundy/25 text-foreground"
                  : "text-muted hover:bg-white/5 hover:text-foreground"
              }`}
            >
              <span className="w-4 text-center opacity-80">{item.icon}</span>
              {item.label}
            </Link>
          );
        })}
      </nav>
    </aside>
  );
}
