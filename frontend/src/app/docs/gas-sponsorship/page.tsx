import Link from "next/link";
import { Prose, Callout, Code, Endpoint, ParamTable } from "@/components/docs/DocsUI";

export default function GasSponsorship() {
  return (
    <Prose>
      <p className="text-xs font-semibold uppercase tracking-wide text-burgundy-bright">
        Essentials
      </p>
      <h1 className="mt-2 text-4xl font-semibold text-foreground">
        Gas sponsorship
      </h1>

      <p>
        Octo can pay the network fees for your users&apos; Stellar transactions
        out of your master wallet, so your users can hold and move stablecoins
        without ever buying XLM for gas. Every sponsored transaction is logged
        with per-wallet spend controls so the feature can&apos;t be abused.
      </p>

      <Callout type="note">
        Read the{" "}
        <Link href="/docs/security">Security model</Link> for the threat
        analysis and signing-path invariants that protect your master wallet
        during sponsorship.
      </Callout>

      <h2>How it works</h2>
      <p>
        Stellar natively supports fee-bump transactions: one account (your
        master wallet) pays the fee for another account&apos;s transaction.
        Here&apos;s the flow:
      </p>
      <ol className="mt-4 list-decimal space-y-2 pl-5 text-sm">
        <li>
          Your user signs a <strong>payment or path-payment transaction XDR</strong>{" "}
          (the inner transaction).
        </li>
        <li>
          Your backend POSTs that XDR to Octo&apos;s{" "}
          <code>/sponsor</code> endpoint.
        </li>
        <li>
          Octo validates the op types, checks your spend limits, wraps the XDR
          in a fee-bump envelope signed by your master wallet, and submits it
          to the Stellar network.
        </li>
        <li>
          Octo returns the outcome immediately and fires a{" "}
          <code>transaction.sponsored</code> webhook when you have endpoints
          registered.
        </li>
      </ol>

      <h2>Enable &amp; configure sponsorship</h2>
      <Endpoint method="PUT" path="/v1/wallets/:id/sponsorship" />
      <p>
        Enable or disable gas sponsorship for a wallet and set per-transaction
        fee caps and the daily budget. Requires a dashboard login token.
      </p>

      <ParamTable
        rows={[
          {
            name: "enabled",
            type: "boolean",
            required: true,
            desc: "Turn sponsorship on or off for this wallet.",
          },
          {
            name: "per_tx_fee_cap_stroops",
            type: "integer",
            desc: "Maximum fee (in stroops) the master wallet will pay per sponsored transaction. Must be ≤ daily_budget_stroops. Defaults to 100\u202f000 (0.01 XLM) if omitted.",
          },
          {
            name: "daily_budget_stroops",
            type: "integer",
            desc: "Maximum total fees (in stroops) the master wallet will spend on sponsorship per calendar day (UTC). Defaults to 100\u202f000\u202f000 (10 XLM) if omitted.",
          },
        ]}
      />

      <Code label="Request">{`curl -X PUT http://localhost:8080/v1/wallets/<WALLET_ID>/sponsorship \\\\
  -H "authorization: Bearer eyJ…" \\\\
  -H "content-type: application/json" \\\\
  -d '{
    "enabled": true,
    "per_tx_fee_cap_stroops": 100000,       // 0.01 XLM
    "daily_budget_stroops": 50000000        // 5 XLM
  }'`}</Code>

      <Code label="Response (200)">{`{
  "statusCode": 200,
  "message": "OK",
  "data": {
    "wallet_id": "52775…",
    "enabled": true,
    "per_tx_fee_cap_stroops": 100000,       // 0.01 XLM
    "daily_budget_stroops": 50000000,       // 5 XLM
    "created_at": "2026-06-15T10:30:00Z",
    "updated_at": "2026-06-15T12:45:00Z"
  }
}`}</Code>

      <p>
        Before any config is saved a wallet is <strong>disabled</strong> with{" "}
        <strong>no caps</strong> (both <code>per_tx_fee_cap_stroops</code> and{" "}
        <code>daily_budget_stroops</code> are <code>null</code>). Set them
        explicitly to bound spend.
      </p>

      <h2>Read current config</h2>
      <Endpoint method="GET" path="/v1/wallets/:id/sponsorship" />
      <p>
        Returns the current sponsorship config (or defaults if none has been
        saved). Requires a dashboard login token.
      </p>

      <Code label="Request">{`curl http://localhost:8080/v1/wallets/<WALLET_ID>/sponsorship \\\\
  -H "authorization: Bearer eyJ…"`}</Code>

      <Code label="Response (200)">{`{
  "statusCode": 200,
  "message": "OK",
  "data": {
    "wallet_id": "52775…",
    "enabled": true,
    "per_tx_fee_cap_stroops": 100000,       // 0.01 XLM
    "daily_budget_stroops": 50000000,       // 5 XLM
    "created_at": "2026-06-15T10:30:00Z",
    "updated_at": "2026-06-15T12:45:00Z"
  }
}`}</Code>

      <h2>Sponsor a transaction</h2>
      <Endpoint method="POST" path="/v1/wallets/:id/sponsor" />
      <p>
        Submit a user-signed inner transaction XDR. Octo validates it, wraps it
        in a fee-bump signed by the master wallet, and submits the result to
        Horizon. Accepts both JWT login tokens and API keys.
      </p>

      <ParamTable
        rows={[
          {
            name: "transaction_xdr",
            type: "string",
            required: true,
            desc: "Base64-encoded TransactionEnvelope XDR of the user's signed inner transaction.",
          },
          {
            name: "max_base_fee_stroops",
            type: "integer",
            required: true,
            desc: "Maximum fee (in stroops) the master wallet will pay for the fee-bump. Must be > 0 and ≤ the per-tx cap.",
          },
        ]}
      />

      <Callout type="tip">
        Only <strong>Payment</strong>, <strong>PathPaymentStrictSend</strong>,
        and <strong>PathPaymentStrictReceive</strong> operations are allowed in
        the inner XDR. Account Merge, Set Options, and all sponsorship-related
        ops are rejected. The inner transaction source must not be the master
        wallet itself.
      </Callout>

      <Code label="Request">{`curl -X POST http://localhost:8080/v1/wallets/<WALLET_ID>/sponsor \\\\
  -H "authorization: Bearer octo_sk_test_abc123…" \\\\
  -H "content-type: application/json" \\\\
  -d '{
    "transaction_xdr": "AAAAAgAAAAD…",
    "max_base_fee_stroops": 100000           // 0.01 XLM
  }'`}</Code>

      <Code label="Response (201 — confirmed)">{`{
  "statusCode": 201,
  "message": "Created",
  "data": {
    "id": "3f1a9b2e…",
    "status": "confirmed",
    "inner_tx_hash": "7f18b2…",
    "fee_bump_tx_hash": "a1c3d4…",
    "fee_stroops": 100000                     // 0.01 XLM
  }
}`}</Code>

      <Code label="Response (201 — on-chain failure)">{`{
  "statusCode": 201,
  "message": "Created",
  "data": {
    "id": "3f1a9b2e…",
    "status": "failed",
    "inner_tx_hash": "7f18b2…",
    "fee_bump_tx_hash": null,
    "fee_stroops": 100000                     // 0.01 XLM
  }
}`}</Code>

      <h2>Sponsored transactions history</h2>
      <Endpoint method="GET" path="/v1/wallets/:id/sponsored-transactions" />
      <p>
        Retrieve the paginated history of sponsored transactions for a wallet.
        Requires a dashboard login token.
      </p>

      <ParamTable
        rows={[
          {
            name: "limit",
            type: "integer",
            desc: "Rows per page (default 50, max 200).",
          },
          {
            name: "status",
            type: "string",
            desc: "Filter by status: pending, confirmed, or failed.",
          },
          {
            name: "before",
            type: "string",
            desc: "Cursor for page-back; use the next_cursor from the previous response.",
          },
        ]}
      />

      <Code label="Request">{`curl "http://localhost:8080/v1/wallets/<WALLET_ID>/sponsored-transactions?limit=50&status=confirmed" \\\\
  -H "authorization: Bearer eyJ…"`}</Code>

      <Code label="Response (200)">{`{
  "statusCode": 200,
  "message": "OK",
  "data": {
    "rows": [
      {
        "id": "3f1a9b2e…",
        "wallet_id": "52775…",
        "inner_tx_hash": "7f18b2…",
        "fee_bump_tx_hash": "a1c3d4…",
        "fee_stroops": 100000,                // 0.01 XLM
        "status": "confirmed",
        "error": null,
        "created_at": "2026-06-15T14:32:00Z"
      }
    ],
    "next_cursor": "3f1a9b2e…"
  }
}`}</Code>

      <h2>Spend controls</h2>
      <p>
        Every wallet has two guardrails that prevent runaway sponsorship costs:
      </p>
      <ul>
        <li>
          <strong>Per-transaction fee cap</strong> (<code>per_tx_fee_cap_stroops</code>)
          — the most the master wallet will ever pay for a single fee-bump. Set
          it low enough that a burst of sponsor requests can&apos;t drain your
          wallet in one go.
        </li>
        <li>
          <strong>Daily budget</strong> (<code>daily_budget_stroops</code>)
          — the total fees the master wallet will pay across all sponsored
          transactions in a UTC calendar day. Each new sponsor request is
          checked against today&apos;s sum of confirmed fees; if it would push
          spending over the budget, Octo returns a 429.
        </li>
      </ul>

      <h3>Worked example</h3>
      <p>Suppose your wallet is configured with:</p>
      <ul>
        <li>Per-tx cap: 100 000 stroops (0.01 XLM)</li>
        <li>Daily budget: 5 000 000 stroops (0.5 XLM)</li>
      </ul>
      <p>
        You can sponsor up to 50 transactions at 0.01 XLM each today. If you
        reach the budget cap before midnight UTC, further sponsor requests
        receive <code>429 Too Many Requests</code> with the error code{" "}
        <code>budget_exceeded</code>. At midnight UTC the counter resets
        automatically.
      </p>

      <Callout type="warning">
        The budget check is an unconditional guard — it rejects the request{" "}
        <strong>before</strong> Octo signs and submits the fee-bump. No sponsor
        request can slip through once the daily budget is exhausted.
      </Callout>

      <h2>Error reference</h2>
      <div className="my-5 overflow-hidden rounded-xl border border-white/10">
        <table className="w-full text-left text-sm">
          <thead className="bg-white/[0.03] text-xs text-muted">
            <tr>
              <th className="px-4 py-2.5">HTTP status</th>
              <th className="px-4 py-2.5">Error code</th>
              <th className="px-4 py-2.5">Meaning</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-white/5">
            <tr>
              <td className="px-4 py-3">
                <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-foreground">
                  400
                </code>
              </td>
              <td className="px-4 py-3 font-mono text-foreground">
                invalid_xdr
              </td>
              <td className="px-4 py-3 text-muted">
                The <code>transaction_xdr</code> is not valid Stellar XDR, or
                it is a fee-bump envelope (not an inner transaction).
              </td>
            </tr>
            <tr>
              <td className="px-4 py-3">
                <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-foreground">
                  400
                </code>
              </td>
              <td className="px-4 py-3 font-mono text-foreground">
                op_not_allowed
              </td>
              <td className="px-4 py-3 text-muted">
                The inner XDR contains an operation type that is not on the
                allowlist (only Payment, PathPaymentStrictSend, and
                PathPaymentStrictReceive are permitted).
              </td>
            </tr>
            <tr>
              <td className="px-4 py-3">
                <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-foreground">
                  400
                </code>
              </td>
              <td className="px-4 py-3 font-mono text-foreground">
                self_sponsorship
              </td>
              <td className="px-4 py-3 text-muted">
                The inner transaction source account matches the master wallet
                — sponsoring yourself is rejected as a security safeguard.
              </td>
            </tr>
            <tr>
              <td className="px-4 py-3">
                <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-foreground">
                  403
                </code>
              </td>
              <td className="px-4 py-3 font-mono text-foreground">
                sponsorship_disabled
              </td>
              <td className="px-4 py-3 text-muted">
                Gas sponsorship is not configured, or is explicitly disabled,
                for this wallet.
              </td>
            </tr>
            <tr>
              <td className="px-4 py-3">
                <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-foreground">
                  409
                </code>
              </td>
              <td className="px-4 py-3 font-mono text-foreground">
                duplicate_inner_tx
              </td>
              <td className="px-4 py-3 text-muted">
                This inner transaction XDR has already been sponsored. Each
                inner transaction hash is unique — re-submitting the same XDR
                is rejected to prevent double-sponsoring.
              </td>
            </tr>
            <tr>
              <td className="px-4 py-3">
                <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-foreground">
                  429
                </code>
              </td>
              <td className="px-4 py-3 font-mono text-foreground">
                budget_exceeded
              </td>
              <td className="px-4 py-3 text-muted">
                The daily sponsorship budget for this wallet would be exceeded
                by this request. The counter resets at midnight UTC.
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <h2>Webhook events</h2>
      <p>
        If you have webhook endpoints registered for a wallet, Octo fires a{" "}
        <code>transaction.sponsored</code> event after every sponsor request
        is finalized. Delivery uses the same HMAC-SHA256 signed POST pattern as
        deposits.
      </p>

      <Code label="POST to your URL">{`X-Octo-Signature: <hmac-sha256 hex>
Content-Type: application/json

{
  "event": "transaction.sponsored",
  "data": {
    "wallet_id": "52775…",
    "inner_tx_hash": "7f18b2…",
    "fee_bump_tx_hash": "a1c3d4…",
    "fee_stroops": 100000,
    "status": "confirmed",
    "created_at": "2026-06-15T14:32:00Z"
  }
}`}</Code>

      <ul>
        <li>
          <code>fee_bump_tx_hash</code> is <code>null</code> when the
          Horizon submission itself fails.
        </li>
        <li>
          Delivery is best-effort and runs asynchronously — the HTTP response
          to the <code>/sponsor</code> call reflects the outcome before the
          webhook fires.
        </li>
        <li>
          Verify the signature the same way you do for deposits; see{" "}
          <Link href="/docs/webhooks">Webhooks</Link> for the verification
          snippet.
        </li>
      </ul>

      <Callout type="tip">
        Treat webhook delivery as a notification, not a source of truth. Always
        reconcile against the completed status in the sponsor response or the{" "}
        <code>GET /sponsored-transactions</code> history.
      </Callout>

      <h2>Audit trail</h2>
      <p>
        Every sponsorship action — config changes, successful submissions,
        on-chain failures, and policy rejections — is recorded in Octo&apos;s
        audit log under the <strong>sponsorship</strong> category. You can
        review these entries on the{" "}
        <Link href="/dashboard/audit">Audit log</Link> page. Rejected requests
        (bad XDR, forbidden op types, budget overruns) are also logged so abuse
        attempts always leave a trace.
      </p>
    </Prose>
  );
}
