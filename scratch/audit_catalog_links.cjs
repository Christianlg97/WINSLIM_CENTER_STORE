const fs = require("node:fs");
const { execFile } = require("node:child_process");
const { promisify } = require("node:util");
const execFileAsync = promisify(execFile);

const catalogPath = "src-tauri/apps.json";
const apps = JSON.parse(fs.readFileSync(catalogPath, "utf8"));
const timeoutMs = 15000;
const concurrency = 8;
const wingetConcurrency = 3;

function targetFor(app) {
  if (app.source_type === "winget" && app.winget_id) {
    return { kind: "winget", url: `${app.winget_source || "winget"}:${app.winget_id}` };
  }
  if (app.source_type === "github_release" && app.github_repo) {
    return { kind: "release", url: `https://github.com/${app.github_repo}/releases/latest` };
  }
  if (app.source_type === "github_repo" && app.github_repo) {
    return { kind: "repository", url: `https://github.com/${app.github_repo}` };
  }
  if (app.download_url) {
    return { kind: app.web_redirect ? "web" : "download", url: app.download_url };
  }
  if (app.source_type === "web" && app.web_url) {
    return { kind: "web", url: app.web_url };
  }
  return null;
}

async function probe(url) {
  const headers = { "user-agent": "WinSlimCenter-Catalog-Audit/1.0" };
  for (const method of ["HEAD", "GET"]) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
      if (method === "GET") headers.range = "bytes=0-0";
      const response = await fetch(url, {
        method,
        headers,
        redirect: "follow",
        signal: controller.signal,
      });
      if (response.ok || response.status === 206 || response.status === 416) {
        await response.body?.cancel();
        return {
          ok: true,
          status: response.status,
          contentType: response.headers.get("content-type") || "",
          finalUrl: response.url,
        };
      }
      if (method === "GET") {
        return { ok: false, status: response.status, error: response.statusText };
      }
    } catch (error) {
      if (method === "GET") return { ok: false, error: String(error) };
    } finally {
      clearTimeout(timer);
    }
  }
  return { ok: false, error: "No response" };
}

async function probeWithCurl(url) {
  try {
    const { stdout } = await execFileAsync(
      "curl.exe",
      [
        "--head", "--location", "--fail", "--silent", "--show-error", "--max-time", "30",
        "-A", "WinSlimCenter-Catalog-Audit/1.0",
        "--write-out", "\nWINSLIM_STATUS:%{http_code}\nWINSLIM_TYPE:%{content_type}\nWINSLIM_URL:%{url_effective}\n",
        url,
      ],
      { encoding: "utf8", timeout: 40000, windowsHide: true, maxBuffer: 2 * 1024 * 1024 },
    );
    const status = Number(stdout.match(/WINSLIM_STATUS:(\d+)/)?.[1] || 0);
    return {
      ok: status >= 200 && status < 400,
      status,
      contentType: stdout.match(/WINSLIM_TYPE:([^\r\n]*)/)?.[1]?.trim() || "",
      finalUrl: stdout.match(/WINSLIM_URL:([^\r\n]*)/)?.[1]?.trim() || url,
    };
  } catch (error) {
    return { ok: false, error: String(error.stderr || error.message || error).trim() };
  }
}

function globMatches(pattern, value) {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replaceAll("*", ".*").replaceAll("?", ".");
  return new RegExp(`^${escaped}$`, "i").test(value);
}

async function probeGithubRelease(app) {
  const pageUrl = `https://github.com/${app.github_repo}/releases/latest`;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const latest = await fetch(pageUrl, {
      headers: { "user-agent": "WinSlimCenter-Catalog-Audit/1.0" },
      redirect: "follow",
      signal: controller.signal,
    });
    if (!latest.ok) return { ok: false, status: latest.status, error: latest.statusText };
    const tag = latest.url.split("/tag/")[1]?.split(/[?#]/)[0];
    if (!tag) return { ok: false, status: latest.status, error: "Latest release has no tag" };
    const assetsUrl = `https://github.com/${app.github_repo}/releases/expanded_assets/${tag}`;
    const assetsResponse = await fetch(assetsUrl, {
      headers: { "user-agent": "WinSlimCenter-Catalog-Audit/1.0" },
      signal: controller.signal,
    });
    if (!assetsResponse.ok) {
      return { ok: false, status: assetsResponse.status, error: "Cannot read release assets" };
    }
    const html = await assetsResponse.text();
    const names = [...html.matchAll(/href="[^"]+\/releases\/download\/[^"]+\/([^"?#]+)"/g)].map((m) =>
      decodeURIComponent(m[1]),
    );
    const matched = app.asset_pattern
      ? names.find((name) => globMatches(app.asset_pattern, name))
      : names.find((name) => /(?:setup|installer|win64|x64|amd64).*(?:\.exe|\.msi|\.msix|\.zip)$/i.test(name));
    if (matched) return { ok: true, status: 200, tag, matched };

    // A repository may publish source-only releases after its latest Windows
    // binary. Match the backend behavior and select the newest release that
    // actually contains the requested Windows asset.
    const releasesResponse = await fetch(
      `https://api.github.com/repos/${app.github_repo}/releases?per_page=20`,
      { headers: { "user-agent": "WinSlimCenter-Catalog-Audit/1.0" }, signal: controller.signal },
    );
    if (releasesResponse.ok) {
      const releases = await releasesResponse.json();
      for (const release of releases) {
        const releaseNames = (release.assets || []).map((asset) => asset.name);
        const releaseMatch = app.asset_pattern
          ? releaseNames.find((name) => globMatches(app.asset_pattern, name))
          : releaseNames.find((name) => /(?:setup|installer|win64|x64|amd64).*(?:\.exe|\.msi|\.msix|\.zip)$/i.test(name));
        if (releaseMatch) {
          return { ok: true, status: 200, tag: release.tag_name, matched: releaseMatch };
        }
      }
    }
    return {
      ok: false,
      status: 200,
      tag,
      error: `Pattern ${app.asset_pattern || "<automatic>"} matched no asset in 20 releases`,
      assets: names,
    };
  } catch (error) {
    return { ok: false, error: String(error) };
  } finally {
    clearTimeout(timer);
  }
}

async function probeWinget(app) {
  const source = app.winget_source || "winget";
  try {
    const { stdout, stderr } = await execFileAsync(
      "winget.exe",
      [
        "show", "--id", app.winget_id, "--exact", "--source", source,
        "--accept-source-agreements", "--disable-interactivity",
      ],
      { encoding: "utf8", timeout: 45000, windowsHide: true, maxBuffer: 4 * 1024 * 1024 },
    );
    const output = `${stdout}\n${stderr}`;
    const installerLine = output
      .split(/\r?\n/)
      .find((line) => /installer\s+url|url\s+(?:del|de)\s+instalador/i.test(line));
    const urls = [...output.matchAll(/https?:\/\/[^\s]+/gi)].map((match) => match[0].replace(/[),.;]+$/, ""));
    const installerUrl = installerLine?.match(/https?:\/\/[^\s]+/i)?.[0]?.replace(/[),.;]+$/, "")
      || [...urls].reverse().find((url) => /(?:\.exe|\.msi|\.msix|\.msixbundle|\.zip)(?:[?#]|$)/i.test(url));
    if (!installerUrl) {
      if (source.toLowerCase() === "msstore" && /tipo de instalador:\s*msstore|installer type:\s*msstore/i.test(output)) {
        return { ok: true, kind: "winget", url: `${source}:${app.winget_id}`, storeManaged: true };
      }
      return { ok: false, kind: "winget", url: `${source}:${app.winget_id}`, error: "WinGet found the package but exposed no installer URL" };
    }
    let download = await probe(installerUrl);
    if (!download.ok) download = await probeWithCurl(installerUrl);
    return {
      kind: "winget",
      url: `${source}:${app.winget_id}`,
      installerUrl,
      ...download,
      htmlDownload: /text\/html|application\/xhtml/i.test(download.contentType || ""),
    };
  } catch (error) {
    return {
      ok: false,
      kind: "winget",
      url: `${source}:${app.winget_id}`,
      error: String(error.stderr || error.stdout || error.message || error).trim(),
    };
  }
}

async function audit(app) {
  const target = targetFor(app);
  if (!target) return { id: app.id, name: app.name, ok: false, error: "No source URL" };
  if (target.kind === "winget") {
    return { id: app.id, name: app.name, ...(await probeWinget(app)) };
  }
  const result = target.kind === "release" ? await probeGithubRelease(app) : await probe(target.url);
  const htmlDownload =
    target.kind === "download" &&
    /text\/html|application\/xhtml/i.test(result.contentType || "");
  return { id: app.id, name: app.name, ...target, ...result, htmlDownload };
}

async function auditIcon(app) {
  const icon = app.icon_url;
  if (!icon) return { id: app.id, ok: false, error: "Missing icon_url" };
  if (!/^https?:/i.test(icon)) {
    const localPath = icon.startsWith("assets/") ? `src/${icon}` : icon;
    return { id: app.id, ok: fs.existsSync(localPath), error: `Missing local icon ${localPath}` };
  }
  let result = await probe(icon);
  if (!result.ok) result = await probeWithCurl(icon);
  const contentType = result.contentType || "";
  const looksLikeImage = /^image\//i.test(contentType)
    || /\.(?:png|jpe?g|webp|gif|svg|ico)(?:[?#]|$)/i.test(result.finalUrl || icon);
  return {
    id: app.id,
    ok: result.ok && looksLikeImage && !/text\/html|application\/xhtml/i.test(contentType),
    error: result.error || (!looksLikeImage ? `Unexpected icon type ${contentType || "unknown"}` : undefined),
    url: icon,
  };
}

async function runPool(items, worker, limit) {
  const results = new Array(items.length);
  let next = 0;
  async function run() {
    while (next < items.length) {
      const index = next++;
      results[index] = await worker(items[index]);
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, run));
  return results;
}

(async () => {
  const ids = new Set();
  const schemaFailures = [];
  for (const [index, app] of apps.entries()) {
    for (const field of ["id", "name", "source_type", "section", "description"]) {
      if (typeof app[field] !== "string" || !app[field].trim()) {
        schemaFailures.push({ id: app.id || `entry-${index + 1}`, error: `Missing or invalid ${field}` });
      }
    }
    if (ids.has(app.id)) schemaFailures.push({ id: app.id, error: "Duplicate id" });
    ids.add(app.id);
  }
  const wingetApps = apps.filter((app) => app.source_type === "winget");
  const otherApps = apps.filter((app) => app.source_type !== "winget");
  const [wingetResults, otherResults] = await Promise.all([
    runPool(wingetApps, audit, wingetConcurrency),
    runPool(otherApps, audit, concurrency),
  ]);
  const byId = new Map([...wingetResults, ...otherResults].map((result) => [result.id, result]));
  const results = apps.map((app) => byId.get(app.id));
  const failures = results.filter((item) => !item.ok || item.htmlDownload);
  const iconResults = await runPool(apps, auditIcon, concurrency);
  const iconFailures = iconResults.filter((item) => !item.ok);
  console.log(`Catalog: ${results.length}`);
  console.log(`Reachable: ${results.length - failures.length}`);
  console.log(`Needs attention: ${failures.length}`);
  for (const item of failures) {
    console.log(
      [item.id, item.status || "ERR", item.htmlDownload ? "HTML_NOT_DOWNLOAD" : item.error || "", item.url]
        .filter(Boolean)
        .join("\t"),
    );
    if (item.assets) console.log(`  assets: ${item.assets.join(", ")}`);
    if (item.installerUrl) console.log(`  installer: ${item.installerUrl}`);
  }
  console.log(`Icons reachable: ${iconResults.length - iconFailures.length}/${iconResults.length}`);
  for (const item of iconFailures) {
    console.log(`${item.id}\tICON\t${item.error || "Invalid icon"}\t${item.url || ""}`);
  }
  for (const item of schemaFailures) console.log(`${item.id}\tSCHEMA\t${item.error}`);
  process.exitCode = failures.length || iconFailures.length || schemaFailures.length ? 1 : 0;
})();
