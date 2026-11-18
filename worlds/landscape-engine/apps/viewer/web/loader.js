// Loading a scene into the viewer, shared by the viewer's page and the editor's
// (docs/editor.md): reads a scene file by HTTP Range, hands the viewer the parts it asks for, and
// finds a GPU adapter to run it on. Nothing here touches the page.

/// The limits the renderer and its fallbacks turn on, as an adapter reports them.
const REPORTED_LIMITS = [
  "maxStorageBuffersPerShaderStage", "maxStorageBuffersInVertexStage",
  "maxStorageBuffersInFragmentStage", "maxStorageTexturesPerShaderStage",
  "maxStorageTexturesInFragmentStage", "maxSampledTexturesPerShaderStage",
  "maxStorageBufferBindingSize", "maxBufferSize", "maxColorAttachments",
  "maxComputeInvocationsPerWorkgroup", "maxComputeWorkgroupSizeX",
  "maxComputeWorkgroupStorageSize", "maxInterStageShaderVariables",
  "maxUniformBufferBindingSize", "maxTextureDimension2D", "maxTextureArrayLayers",
];

/// The browser's own adapter request, kept before `requestAdapter` may replace it, so that the
/// report says what the browser itself offers.
const browserRequestAdapter = globalThis.GPU?.prototype.requestAdapter;
let compatibilityShim = false;

/// What the browser offers, adapter by adapter: for the page to show when the viewer can't
/// start, as a phone's console is out of reach. Each adapter is asked for afresh.
export async function adapterReport() {
  const lines = [
    `Page: ${location.href}`,
    `Browser: ${navigator.userAgent}`,
    `Secure context: ${isSecureContext}, WebGPU: ${Boolean(navigator.gpu)}`
      + (compatibilityShim ? ", using a compatibility adapter as core" : ""),
  ];
  const describe = async (label, options) => {
    let adapter;
    try {
      adapter = await browserRequestAdapter.call(navigator.gpu, options);
    } catch (error) {
      lines.push(`${label}: request failed (${error.message ?? error})`);
      return;
    }
    if (!adapter) {
      lines.push(`${label}: none`);
      return;
    }
    const info = adapter.info ?? {};
    const name = [info.vendor, info.architecture, info.device, info.description]
      .filter(Boolean).join(" / ");
    lines.push(`${label}: ${name || "unnamed"}${info.isFallbackAdapter ? " (software)" : ""}`);
    lines.push(`  features: ${[...adapter.features].sort().join(", ") || "none"}`);
    lines.push(`  limits: ${REPORTED_LIMITS.filter((key) => key in adapter.limits)
      .map((key) => `${key} ${adapter.limits[key]}`).join(", ")}`);
  };
  if (navigator.gpu) {
    await describe("Core adapter", undefined);
    await describe("Compatibility adapter", { featureLevel: "compatibility" });
    await describe("Software adapter", { forceFallbackAdapter: true });
  }
  return lines.join("\n");
}

/// Reads parts of a scene file by HTTP Range. If the server doesn't do ranges, the whole file
/// comes down once, and ranges are served from memory, so any static host works.
export class SceneFile {
  bytesFetched = 0;
  #whole = null;
  #etag = null;
  #url;
  #onDownload;

  /// `onDownload(received, total)` follows a whole-file download, for a server without ranges;
  /// `total` is 0 if the server doesn't say, and `received` is null when it has finished.
  constructor(url, onDownload = () => {}) {
    this.#url = url;
    this.#onDownload = onDownload;
  }

  /// A scene file already in memory, as the editor's bake gives it.
  static fromBytes(bytes) {
    const file = new SceneFile("memory");
    file.#whole = bytes;
    file.bytesFetched = bytes.length;
    return file;
  }

  async read(start, end, signal) {
    if (this.#whole) return this.#whole.subarray(start, end);
    const response = await fetch(this.#url, { headers: { Range: `bytes=${start}-${end - 1}` }, signal });
    if (!response.ok) {
      throw new Error(`Could not load ${this.#url} (HTTP ${response.status}).`);
    }
    if (response.status === 200) {
      this.#whole = await downloadWhole(response, this.#onDownload);
      this.bytesFetched = this.#whole.length;
      return this.#whole.subarray(start, end);
    }
    // A scene replaced while it's being read would mix two files, so start again from the top.
    const etag = response.headers.get("etag");
    if (this.#etag && etag && etag !== this.#etag) {
      location.reload();
      return new Promise(() => {});
    }
    this.#etag = etag;
    const bytes = new Uint8Array(await response.arrayBuffer());
    this.bytesFetched += bytes.length;
    return bytes;
  }
}

/// Downloads a whole file with progress, for servers without Range support.
async function downloadWhole(response, onDownload) {
  const total = Number(response.headers.get("content-length")) || 0;
  const reader = response.body.getReader();
  const chunks = [];
  let received = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.length;
    onDownload(received, total);
  }
  const bytes = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  onDownload(null, total);
  return bytes;
}

export async function inflate(bytes) {
  const unpacked = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  return new Uint8Array(await new Response(unpacked).arrayBuffer());
}

/// Unpacks a block that should come to `limit` bytes, refusing to unpack more: a scene file comes
/// from outside.
export async function inflateAtMost(bytes, limit) {
  const unpacked = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  const reader = unpacked.getReader();
  const out = new Uint8Array(limit);
  let filled = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (filled + value.length > limit) {
      await reader.cancel();
      throw new Error("a part of the scene unpacks larger than it says");
    }
    out.set(value, filled);
    filled += value.length;
  }
  return out.subarray(0, filled);
}

/// How long one try at reading part of a scene file may take, stalled or not.
const READ_TIMEOUT_MS = 60000;

/// Reads part of a scene file, trying a few times before giving up.
export async function readWithRetries(file, start, end) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await file.read(start, end, AbortSignal.timeout(READ_TIMEOUT_MS));
    } catch (error) {
      if (attempt === 3) throw error;
      await new Promise((resolve) => setTimeout(resolve, 500 * attempt));
    }
  }
}

/// Runs in workers: fetches one run of a scene file and unpacks its pages, off the main thread,
/// so that streaming doesn't cost the frame rate.
const FETCH_WORKER = `
async function inflate(bytes) {
  const unpacked = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  return new Uint8Array(await new Response(unpacked).arrayBuffer());
}
// Many parts of a file in one response, one after another: scripts/serve.py.
async function many(url, spans) {
  if (spans.length === 0) return new Uint8Array(0);
  const ranges = spans.map(([start, end]) => start + "-" + (end - 1)).join(",");
  const response = await fetch(url, { headers: { "X-Solstice-Ranges": ranges } });
  if (!response.headers.get("X-Solstice-Ranges")) {
    throw new Error("this scene needs scripts/serve.py, which can send many parts at once");
  }
  return new Uint8Array(await response.arrayBuffer());
}
onmessage = async ({ data: { url, tablesUrl, ticket, run } }) => {
  try {
    const withRecords = run.filter((request) => request.clustersEnd > request.clustersStart);
    const [stored, records] = await Promise.all([
      many(url, run.map((request) => [request.start, request.end])),
      many(tablesUrl, withRecords.map((request) => [request.clustersStart, request.clustersEnd])),
    ]);
    let at = 0;
    let recordsAt = 0;
    const pages = await Promise.all(run.map(async (request) => {
      const part = stored.subarray(at, at + request.end - request.start);
      at += request.end - request.start;
      let clusters = null;
      if (request.clustersEnd > request.clustersStart) {
        const length = request.clustersEnd - request.clustersStart;
        clusters = records.slice(recordsAt, recordsAt + length);
        recordsAt += length;
      }
      return { id: request.id, data: request.deflated ? await inflate(part) : part.slice(), clusters };
    }));
    postMessage({ ticket, pages }, pages.flatMap((page) =>
      page.clusters ? [page.data.buffer, page.clusters.buffer] : [page.data.buffer]));
  } catch (error) {
    postMessage({ ticket, failed: run.map((request) => request.id), error: String(error) });
  }
};
`;

/// Fetches runs of pages in workers, each going to whichever has the least to do.
class WorkerFetcher {
  #workers;
  #busy;
  #tickets = new Map();
  #next = 0;
  #url;
  #tablesUrl;

  constructor(url, tablesUrl, count, onPages, onFailed) {
    this.#url = url;
    this.#tablesUrl = new URL(tablesUrl, location.href).href;
    const script = URL.createObjectURL(new Blob([FETCH_WORKER], { type: "text/javascript" }));
    this.#busy = new Array(count).fill(0);
    this.#workers = this.#busy.map((_, index) => {
      const worker = new Worker(script);
      worker.onmessage = ({ data }) => {
        this.#busy[index]--;
        this.#tickets.delete(data.ticket);
        if (data.failed) {
          console.warn(`Could not fetch part of the scene, and will retry: ${data.error}`);
          onFailed(data.failed);
        } else {
          onPages(data.pages);
        }
      };
      return worker;
    });
  }

  fetch(run) {
    const index = this.#busy.indexOf(Math.min(...this.#busy));
    this.#busy[index]++;
    const ticket = this.#next++;
    this.#tickets.set(ticket, run);
    this.#workers[index].postMessage({
      url: new URL(this.#url, location.href).href, tablesUrl: this.#tablesUrl, ticket, run,
    });
  }
}

/// Fetches what the viewer asks for, keeping the page responsive while it does.
export class Streamer {
  // Browsers run about six connections per host over HTTP/1.1, so requests for parts of the file
  // that are close together share one, as long as what lies between them is small.
  static MERGE_GAP_BYTES = 256 * 1024;
  static MERGE_MAX_BYTES = 8 * 1048576;
  static RETRY_MS = 1000;
  #viewer;
  #file;
  #workers = null;
  #tablesStart;
  // Pages being fetched at once.
  #maxFetching;
  // Pages asked for in one request, by the workers. Six requests at a time is all a browser sends.
  #pagesPerRequest;

  /// `split`, for a scene read in parts, names its scene file and its `.web` tables file, whose
  /// pages `workers` workers fetch; it needs `scripts/serve.py`.
  constructor(viewer, file, tablesStart = 0, { maxFetching, workers, pagesPerRequest, split = null } = {}) {
    this.#viewer = viewer;
    this.#file = file;
    this.#tablesStart = tablesStart;
    this.#maxFetching = maxFetching || (split ? 1024 : 16);
    this.#pagesPerRequest = pagesPerRequest || 64;
    // Only for a scene read in parts, which is served with Range requests by something known.
    if (split) {
      this.#workers = new WorkerFetcher(
        split.url,
        split.tablesUrl,
        workers || Math.min(8, navigator.hardwareConcurrency || 4),
        (pages) => {
          for (const { id, data, clusters } of pages) {
            if (clusters) this.#viewer.deliver_page(id, data, clusters);
            else this.#viewer.deliver(id, data);
          }
        },
        (ids) => { for (const id of ids) setTimeout(() => this.#viewer.request_failed(id), Streamer.RETRY_MS); },
      );
    }
  }

  /// Call once per frame.
  pump() {
    const flat = this.#viewer.stream_requests(this.#maxFetching);
    const requests = [];
    for (let i = 0; i < flat.length; i += 6) {
      requests.push({
        id: flat[i], start: flat[i + 1], end: flat[i + 2], deflated: flat[i + 3] !== 0,
        clustersStart: this.#tablesStart + flat[i + 4], clustersEnd: this.#tablesStart + flat[i + 5],
      });
    }
    requests.sort((a, b) => a.start - b.start);
    if (this.#workers) {
      for (let i = 0; i < requests.length; i += this.#pagesPerRequest) {
        this.#workers.fetch(requests.slice(i, i + this.#pagesPerRequest));
      }
      return;
    }
    let run = [];
    for (const request of requests) {
      const last = run[run.length - 1];
      if (last && (request.start - last.end > Streamer.MERGE_GAP_BYTES
        || request.end - run[0].start > Streamer.MERGE_MAX_BYTES)) {
        this.#fetchRun(run);
        run = [];
      }
      run.push(request);
    }
    if (run.length > 0) this.#fetchRun(run);
  }

  async #fetchRun(run) {
    const start = run[0].start;
    const end = run[run.length - 1].end;
    let stored;
    try {
      stored = await this.#file.read(start, end);
    } catch (error) {
      console.warn(`Could not fetch part of the scene, and will retry: ${error.message ?? error}`);
      for (const request of run) {
        setTimeout(() => this.#viewer.request_failed(request.id), Streamer.RETRY_MS);
      }
      return;
    }
    for (const request of run) this.#unpack(request, stored.subarray(request.start - start, request.end - start));
  }

  async #unpack(request, stored) {
    try {
      this.#viewer.deliver(request.id, request.deflated ? await inflate(stored) : stored);
    } catch (error) {
      console.warn(`Could not unpack part of the scene, and will retry: ${error.message ?? error}`);
      setTimeout(() => this.#viewer.request_failed(request.id), Streamer.RETRY_MS);
    }
  }
}

/// Hands over tables too large for a browser's memory a part at a time, each going on to the GPU.
/// `onProgress(done, total)` follows it, in bytes.
export async function loadExternalTables(viewer, tables, start, onProgress = () => {}) {
  const sizes = viewer.external_tables();
  if (sizes.length === 0) return;
  const [clusterCount, pinnedBytes] = sizes;
  const CHUNK = 64 * 1048576;
  const RECORD = 48;
  const pageSizes = viewer.pinned_sizes();
  const recordCounts = viewer.pinned_cluster_counts();
  let done = 0;
  const report = () => onProgress(done, pinnedBytes);
  // Lets the GPU process catch up, so uploads don't pile up in memory.
  const breathe = () => new Promise((resolve) => setTimeout(resolve, 0));

  // The table holds each page's records in turn, and pinned pages come first.
  const pinnedRecords = recordCounts.reduce((sum, count) => sum + count, 0) * RECORD;
  const records = await readWithRetries(tables, start, start + pinnedRecords);
  const pinnedStart = start + clusterCount * RECORD;
  let page = 0;
  let at = 0;
  let recordsAt = 0;
  while (page < pageSizes.length) {
    let end = at;
    let last = page;
    while (last < pageSizes.length && (end === at || end + pageSizes[last] - at <= CHUNK)) {
      end += pageSizes[last];
      last++;
    }
    const bytes = await readWithRetries(tables, pinnedStart + at, pinnedStart + end);
    let inside = 0;
    for (; page < last; page++) {
      const length = recordCounts[page] * RECORD;
      viewer.install_pinned(
        page,
        bytes.subarray(inside, inside + pageSizes[page]),
        records.subarray(recordsAt, recordsAt + length),
      );
      inside += pageSizes[page];
      recordsAt += length;
    }
    done += end - at;
    at = end;
    report();
    await breathe();
  }
  viewer.finish_external_tables();
}

/// The adapter the viewer will be given. Chrome offers some GPUs only to pages asking for
/// WebGPU's "compatibility" feature level, though they can do all of core WebGPU, as its own
/// software adapter on an Android emulator does; the viewer asks for core. If no core adapter
/// is offered, such an adapter is taken, and its devices asked for core. Anywhere a core
/// adapter is offered, nothing changes.
export async function requestAdapter() {
  const adapter = await navigator.gpu.requestAdapter();
  if (adapter) return adapter;
  const CORE = "core-features-and-limits";
  const compatible = await navigator.gpu.requestAdapter({ featureLevel: "compatibility" });
  if (!compatible?.features.has(CORE)) return null;
  const requestAdapter = GPU.prototype.requestAdapter;
  GPU.prototype.requestAdapter = function (options = {}) {
    return requestAdapter.call(this, { ...options, featureLevel: "compatibility" });
  };
  const requestDevice = GPUAdapter.prototype.requestDevice;
  GPUAdapter.prototype.requestDevice = function (descriptor = {}) {
    const requiredFeatures = [...(descriptor.requiredFeatures ?? []), CORE];
    return requestDevice.call(this, { ...descriptor, requiredFeatures });
  };
  console.info("using a GPU offered for WebGPU compatibility, which can do core WebGPU");
  compatibilityShim = true;
  return compatible;
}

/// Opens a scene file: reads its header and manifest, makes a viewer on `canvas` from them, hands
/// over any tables that come separately, and gives back the viewer, a streamer to pump once a
/// frame, and the file. `wasm` is the viewer's module (`WebViewer`, `header_bytes`,
/// `read_header`). `options`: `url`; `tablesUrl` for a scene read in parts, whose manifest comes
/// from its `.web` file, which leaves out the tables too large for a browser's memory and
/// carries them after it (pages still come from `url`); `geometryBudget`, `textureBudget`,
/// `unpackTextures`, `defaultLimits` as `WebViewer.create` takes them; `streaming`, the
/// streamer's options; and `onDownload`, `onTablesProgress` and `onStage` to follow it.
export async function openScene(wasm, canvas, options) {
  const { WebViewer, header_bytes, read_header } = wasm;
  // `bytes` opens a scene file already in memory instead of `url`.
  const file = options.bytes
    ? SceneFile.fromBytes(options.bytes)
    : new SceneFile(options.url, options.onDownload);
  const tables = options.tablesUrl ? new SceneFile(options.tablesUrl, options.onDownload) : file;
  const [manifestOffset, manifestBytes] = read_header(await tables.read(0, header_bytes()));
  const manifestEnd = manifestOffset + manifestBytes;
  const manifest = await inflate(await tables.read(manifestOffset, manifestEnd));
  const [pagesOffset, pagesManifestBytes] = tables === file
    ? [manifestOffset, manifestBytes]
    : read_header(await file.read(0, header_bytes()));
  options.onStage?.("Preparing the GPU…");
  const viewer = await WebViewer.create(
    canvas,
    manifest,
    pagesOffset + pagesManifestBytes,
    options.geometryBudget,
    options.textureBudget,
    Boolean(options.unpackTextures),
    Boolean(options.defaultLimits),
  );
  // A GPU given less than the renderer asks for has passes that do without.
  const fallbacks = viewer.fallbacks();
  if (fallbacks) console.info(`fallbacks for this GPU's limits: ${fallbacks}`);
  await loadExternalTables(viewer, tables, manifestEnd, options.onTablesProgress);
  const streamer = new Streamer(viewer, file, manifestEnd, {
    ...options.streaming,
    split: options.tablesUrl ? { url: options.url, tablesUrl: options.tablesUrl } : null,
  });
  return { viewer, streamer, file };
}
