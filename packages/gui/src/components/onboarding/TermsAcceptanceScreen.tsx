import React from "react";
import Button from "@/components/ui/Button";
import { CheckCircle2, ExternalLink, FileText } from "@/lib/icons";

const TOS_URL = "https://bifurc.harshalkudale.com/tos";

interface TermsAcceptanceScreenProps {
  onAccept: () => void;
}

export default function TermsAcceptanceScreen({
  onAccept,
}: TermsAcceptanceScreenProps) {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <div className="mx-auto flex min-h-screen w-full max-w-6xl items-center px-6 py-8">
        <div className="grid w-full overflow-hidden rounded-3xl border border-border bg-surface shadow-[var(--glow-signal-md)] lg:grid-cols-[0.9fr_1.1fr]">

          {/* Left side — context / branding */}
          <div className="relative flex min-h-[320px] flex-col justify-between border-b border-border p-8 sm:p-10 lg:min-h-[560px] lg:border-b-0 lg:border-r">
            <div>
              <div className="mb-10 flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-signal/30 bg-signal/10 text-signal">
                  <span className="text-lg font-semibold">B</span>
                </div>

                <div>
                  <div className="text-sm font-semibold">Bifurc</div>
                  <div className="text-xs text-muted-foreground">
                    Local development workspace
                  </div>
                </div>
              </div>

              <div className="max-w-sm">
                <div className="mb-4 text-xs font-medium uppercase tracking-[0.18em] text-signal">
                  Before you begin
                </div>

                <h1 className="text-3xl font-semibold tracking-tight sm:text-4xl">
                  A few things before we get started.
                </h1>

                <p className="mt-5 text-sm leading-7 text-muted-foreground">
                  Bifurc is a local-first development workspace designed to
                  bring your tools and workflows together in one place.
                </p>
              </div>
            </div>

            <div className="mt-12 grid gap-3 sm:grid-cols-3 lg:grid-cols-1">
              <InfoItem
                title="Local-first"
                description="Your development workflow stays in your environment."
              />

              <InfoItem
                title="Your workspace"
                description="Manage services, requests, proxies, mocks and more."
              />

              <InfoItem
                title="Built for developers"
                description="Powerful tools without unnecessary complexity."
              />
            </div>
          </div>

          {/* Right side — action */}
          <div className="flex flex-col justify-center p-8 sm:p-10 lg:p-14">
            <div className="mx-auto w-full max-w-lg">
              <div className="mb-8 inline-flex h-12 w-12 items-center justify-center rounded-2xl border border-signal/30 bg-signal/10 text-signal">
                <FileText size={20} />
              </div>

              <div>
                <div className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                  Terms of Service
                </div>

                <h2 className="mt-3 text-2xl font-semibold tracking-tight sm:text-3xl">
                  Review and accept the terms
                </h2>

                <p className="mt-4 text-sm leading-7 text-muted-foreground">
                  Before using Bifurc, please review our Terms of Service.
                  Accepting them allows you to continue into the application.
                </p>
              </div>

              {/* Terms summary */}
              <div className="mt-8 rounded-2xl border border-border bg-card/70">
                <div className="border-b border-border px-5 py-4">
                  <div className="flex items-center gap-3">
                    <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-border bg-background text-muted-foreground">
                      <FileText size={15} />
                    </div>

                    <div>
                      <div className="text-sm font-medium">
                        Bifurc Terms of Service
                      </div>
                      <div className="mt-0.5 text-xs text-muted-foreground">
                        Please review before continuing
                      </div>
                    </div>
                  </div>
                </div>

                <div className="px-5 py-5 text-sm leading-6 text-muted-foreground">
                  By continuing, you confirm that you have reviewed the latest
                  Terms of Service and agree to use Bifurc under those terms.
                </div>
              </div>

              {/* Actions */}
              <div className="mt-6 grid gap-3 sm:grid-cols-2">
                <Button
                  variant="secondary"
                  className="w-full justify-center"
                  icon={<ExternalLink size={14} />}
                  onClick={() => window.api.openExternal(TOS_URL)}
                >
                  Read Terms
                </Button>

                <Button
                  variant="primary"
                  className="w-full justify-center"
                  icon={<CheckCircle2 size={14} />}
                  onClick={onAccept}
                >
                  Accept and continue
                </Button>
              </div>

              <p className="mt-6 text-center text-xs leading-5 text-muted-foreground">
                You can review the Terms of Service again at any time from our
                website.
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function InfoItem({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="rounded-xl border border-border bg-card/40 px-4 py-3">
      <div className="text-sm font-medium">{title}</div>
      <div className="mt-1 text-xs leading-5 text-muted-foreground">
        {description}
      </div>
    </div>
  );
}
