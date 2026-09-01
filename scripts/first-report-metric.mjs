const identifier = /^[A-Za-z0-9_.-]+$/;

function requireIdentifier(value, field) {
  if (typeof value !== "string" || !identifier.test(value)) {
    throw new Error(`${field} must be a bounded identifier`);
  }
  return value;
}

/**
 * Execute the versioned first-report activation contract over metadata-only
 * local observations. Each observation represents one unique signed workspace
 * ledger and either its first elapsed time or a missing first-report outcome.
 */
export function summarizeFirstReportActivation(contract, observations) {
  if (contract.schemaVersion !== 1) throw new Error("unsupported product metric schema");
  const metric = contract.activation;
  if (metric.metricId !== "first-saved-report-within-ten-minutes-v1") {
    throw new Error("unsupported first-report metric");
  }
  const maximum = metric.numerator.maximumElapsedMilliseconds;
  const target = metric.target.rate;
  if (maximum !== 600_000 || target !== 0.8) {
    throw new Error("first-report threshold or target drifted from version 1");
  }

  const allowedFamilies = new Set(metric.requiredPlatformFamilies);
  const seenWorkspaces = new Set();
  const groups = new Map();
  for (const observation of observations) {
    const workspaceId = requireIdentifier(observation.workspaceId, "workspaceId");
    if (seenWorkspaces.has(workspaceId)) throw new Error("workspace appears more than once");
    seenWorkspaces.add(workspaceId);
    const productVersion = requireIdentifier(observation.productVersion, "productVersion");
    const platform = requireIdentifier(observation.platform, "platform");
    const cohort = requireIdentifier(observation.cohort, "cohort");
    const platformFamily = platform.split("-", 1)[0];
    if (!allowedFamilies.has(platformFamily)) continue;
    const key = `${productVersion}\u0000${platformFamily}\u0000${cohort}`;
    const group = groups.get(key) ?? {
      metricId: metric.metricId,
      productVersion,
      platformFamily,
      cohort,
      eligibleWorkspaces: 0,
      qualifiedWorkspaces: 0,
    };
    group.eligibleWorkspaces += 1;
    if (observation.elapsedMilliseconds !== null && (
      !Number.isSafeInteger(observation.elapsedMilliseconds) || observation.elapsedMilliseconds < 0
    )) {
      throw new Error("elapsedMilliseconds must be a non-negative integer or null");
    }
    if (observation.elapsedMilliseconds !== null && observation.elapsedMilliseconds <= maximum) {
      group.qualifiedWorkspaces += 1;
    }
    groups.set(key, group);
  }

  return [...groups.values()]
    .map((group) => {
      const successRate = group.qualifiedWorkspaces / group.eligibleWorkspaces;
      return { ...group, maximumElapsedMilliseconds: maximum, targetRate: target, successRate, targetMet: successRate >= target };
    })
    .sort((left, right) =>
      [left.productVersion, left.platformFamily, left.cohort]
        .join("\u0000")
        .localeCompare([right.productVersion, right.platformFamily, right.cohort].join("\u0000")),
    );
}
