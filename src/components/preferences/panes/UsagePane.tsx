import React from 'react'
import { Loader2, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import {
  useCodexCliAuth,
  useCodexCliStatus,
  useCodexUsage,
} from '@/services/codex-cli'
import { useGeminiCliAuth, useGeminiCliStatus, useGeminiUsage } from '@/services/gemini-cli'

interface UsageWindow {
  usedPercent: number
  resetsAt: number | null
}

const SettingsSection: React.FC<{
  title: string
  children: React.ReactNode
}> = ({ title, children }) => (
  <div className="space-y-4">
    <div>
      <h3 className="text-lg font-medium text-foreground">{title}</h3>
      <Separator className="mt-2" />
    </div>
    {children}
  </div>
)

const UsageRow: React.FC<{
  label: string
  usage: UsageWindow | null
}> = ({ label, usage }) => {
  if (!usage) return null

  const usedPercent = Math.max(0, Math.min(100, usage.usedPercent))
  const resetsAtLabel = usage.resetsAt
    ? new Date(usage.resetsAt * 1000).toLocaleString()
    : null

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-sm">
        <span className="text-foreground">{label}</span>
        <span className="text-muted-foreground">{usedPercent.toFixed(1)}%</span>
      </div>
      <div className="h-2 w-full rounded-full bg-secondary">
        <div
          className="h-2 rounded-full bg-primary transition-[width] duration-300"
          style={{ width: `${usedPercent}%` }}
        />
      </div>
      {resetsAtLabel && (
        <p className="text-xs text-muted-foreground">Resets: {resetsAtLabel}</p>
      )}
    </div>
  )
}

function getQueryErrorMessage(error: unknown, fallback: string) {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  if (
    error &&
    typeof error === 'object' &&
    'message' in error &&
    typeof error.message === 'string'
  ) {
    return error.message
  }
  return fallback
}

export const UsagePane: React.FC = () => {
  const codexStatus = useCodexCliStatus()
  const codexAuth = useCodexCliAuth({ enabled: !!codexStatus.data?.installed })
  const codexUsage = useCodexUsage({
    enabled: !!codexStatus.data?.installed && !!codexAuth.data?.authenticated,
  })
  
  const geminiStatus = useGeminiCliStatus()
  const geminiAuth = useGeminiCliAuth({
    enabled: !!geminiStatus.data?.installed,
  })
  const geminiUsage = useGeminiUsage({
    enabled: !!geminiStatus.data?.installed && !!geminiAuth.data?.authenticated,
  })

  const codexErrorMessage = getQueryErrorMessage(
    codexUsage.error,
    'Failed to load Codex usage.'
  )
  const geminiErrorMessage = getQueryErrorMessage(
    geminiUsage.error,
    'Failed to load Gemini usage.'
  )
  
  // Group Gemini quotas by family
  const groupedGeminiQuotas = React.useMemo(() => {
    if (!geminiUsage.data?.quotas) return []
    
    const families: Record<string, GeminiUsageBucket> = {}
    
    geminiUsage.data.quotas.forEach(q => {
      let family = 'Other'
      const id = q.model_id.toLowerCase()
      if (id.includes('pro')) family = 'Pro Family'
      else if (id.includes('flash') && id.includes('lite')) family = 'Flash Lite Family'
      else if (id.includes('flash')) family = 'Flash Family'
      
      // If models share the same pool, their metrics will be identical.
      // We take the one with highest usage just in case.
      if (!families[family] || q.usage_percent > families[family].usage_percent) {
        families[family] = {
          ...q,
          model_id: family
        }
      }
    })
    
    // Sort families: Pro, Flash, Lite
    const order = ['Pro Family', 'Flash Family', 'Flash Lite Family']
    return Object.values(families).sort((a, b) => {
      const idxA = order.indexOf(a.model_id)
      const idxB = order.indexOf(b.model_id)
      if (idxA !== -1 && idxB !== -1) return idxA - idxB
      if (idxA !== -1) return -1
      if (idxB !== -1) return 1
      return a.model_id.localeCompare(b.model_id)
    })
  }, [geminiUsage.data?.quotas])

  const isGeminiIdentityKnown = !!(geminiAuth.data?.email || geminiAuth.data?.project || geminiUsage.data?.plan_name || geminiUsage.data?.tier)

  const isRefreshing =
    codexUsage.isFetching ||
    codexAuth.isFetching ||
    geminiUsage.isFetching ||
    geminiAuth.isFetching

  return (
    <div className="space-y-6">
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2">
        <div className="flex items-center justify-between gap-3 text-xs">
          <span className="text-muted-foreground">
            Usage data auto-refreshes every 5 minutes.
          </span>
          <span className="inline-flex items-center gap-1.5 text-muted-foreground">
            {isRefreshing ? (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Refreshing...
              </>
            ) : (
              <>
                <RefreshCw className="h-3.5 w-3.5" />
                Up to date
              </>
            )}
          </span>
        </div>
      </div>

      <SettingsSection title="Claude">
        <p className="text-sm text-muted-foreground">
          Claude usage tracking is temporarily disabled due to an authentication bug that causes repeated logouts.
        </p>
      </SettingsSection>

      <SettingsSection title="Codex">
        {!codexStatus.data?.installed ? (
          <p className="text-sm text-muted-foreground">
            Codex CLI is not installed.
          </p>
        ) : codexAuth.isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Checking authentication...
          </div>
        ) : !codexAuth.data?.authenticated ? (
          <p className="text-sm text-muted-foreground">
            Codex is not authenticated. Run `codex` in your terminal to log in.
          </p>
        ) : codexUsage.isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading usage...
          </div>
        ) : codexUsage.isError ? (
          <div className="space-y-3">
            <p className="text-sm text-destructive">{codexErrorMessage}</p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => codexUsage.refetch()}
            >
              <RefreshCw className="h-3.5 w-3.5" />
              Retry
            </Button>
          </div>
        ) : codexUsage.data ? (
          <div className="space-y-5">
            <div className="rounded-md border border-border p-3">
              <p className="text-xs text-muted-foreground">Plan</p>
              <p className="text-sm font-medium text-foreground">
                {codexUsage.data.planType ?? 'Unknown'}
              </p>
              {codexUsage.data.creditsRemaining !== null && (
                <p className="mt-1 text-xs text-muted-foreground">
                  Credits remaining: {codexUsage.data.creditsRemaining}
                </p>
              )}
            </div>

            <UsageRow label="Session" usage={codexUsage.data.session} />
            <UsageRow label="Weekly" usage={codexUsage.data.weekly} />
            <UsageRow label="Reviews" usage={codexUsage.data.reviews} />

            {codexUsage.data.modelLimits.length > 0 && (
              <div className="space-y-3">
                <p className="text-sm font-medium text-foreground">
                  Additional Limits
                </p>
                {codexUsage.data.modelLimits.map(limit => (
                  <div
                    key={limit.label}
                    className="space-y-2 rounded-md border border-border p-3"
                  >
                    <p className="text-sm text-foreground">{limit.label}</p>
                    <UsageRow label="Session" usage={limit.session} />
                    <UsageRow label="Weekly" usage={limit.weekly} />
                  </div>
                ))}
              </div>
            )}

            <p className="text-xs text-muted-foreground">
              Last updated:{' '}
              {new Date(codexUsage.data.fetchedAt * 1000).toLocaleString()}
            </p>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">No usage data available.</p>
        )}
      </SettingsSection>

      <SettingsSection title="Gemini">
        {!geminiStatus.data?.installed ? (
          <p className="text-sm text-muted-foreground">
            Gemini CLI is not installed.
          </p>
        ) : geminiAuth.isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Checking authentication...
          </div>
        ) : !geminiAuth.data?.authenticated ? (
          <p className="text-sm text-muted-foreground">
            Gemini CLI is not authenticated. Run `gemini -i "/auth signin"` in your terminal.
          </p>
        ) : geminiUsage.isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading usage...
          </div>
        ) : geminiUsage.isError ? (
          <div className="space-y-3">
            <p className="text-sm text-destructive">{geminiErrorMessage}</p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => geminiUsage.refetch()}
            >
              <RefreshCw className="h-3.5 w-3.5" />
              Retry
            </Button>
          </div>
        ) : geminiUsage.data ? (
          <div className="space-y-5">
            {isGeminiIdentityKnown && (
              <div className="rounded-md border border-border p-3 space-y-2">
                {geminiAuth.data?.email && (
                  <div>
                    <p className="text-xs text-muted-foreground">Account</p>
                    <p className="text-sm font-medium text-foreground">
                      {geminiAuth.data.email}
                    </p>
                  </div>
                )}
                {geminiAuth.data?.project && (
                  <div>
                    <p className="text-xs text-muted-foreground">GCP Project</p>
                    <p className="text-sm font-medium text-foreground">
                      {geminiAuth.data.project}
                    </p>
                  </div>
                )}
                {(geminiUsage.data.plan_name || geminiUsage.data.tier) && (
                  <div>
                    <p className="text-xs text-muted-foreground">Plan</p>
                    <p className="text-sm font-medium text-foreground">
                      {geminiUsage.data.plan_name ?? geminiUsage.data.tier}
                    </p>
                  </div>
                )}
              </div>
            )}

            {groupedGeminiQuotas.length > 0 ? (
              <div className="space-y-4">
                {groupedGeminiQuotas.map(quota => (
                  <UsageRow
                    key={quota.model_id}
                    label={quota.model_id}
                    usage={{
                      usedPercent: quota.usage_percent,
                      resetsAt: quota.reset_time,
                    }}
                  />
                ))}
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">No quota buckets reported.</p>
            )}

            <p className="text-xs text-muted-foreground">
              Last updated:{' '}
              {new Date(geminiUsage.data.fetched_at * 1000).toLocaleString()}
            </p>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">No usage data available.</p>
        )}
      </SettingsSection>
    </div>
  )
}
