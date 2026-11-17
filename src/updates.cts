import fs = require('node:fs/promises');
import path = require('node:path');
import semver = require('semver');

export type UpdateTrack = 'stable' | 'beta';
export interface UpdateStatus { track: UpdateTrack; phase: 'idle' | 'checking' | 'downloading' | 'ready' | 'restarting' | 'error' | 'disabled'; message: string }
interface Release { tag_name: string; draft: boolean; prerelease: boolean; assets: { name: string }[] }
const CHECK_INTERVAL_MS = 10 * 60 * 1000;
const repository = 'Savage1001/refract';

function selectRelease(releases: Release[], track: UpdateTrack, current: string): Release | undefined {
  return releases.filter(release => {
    const version = semver.valid(release.tag_name);
    if (!version || release.draft || !semver.gt(version, current)) return false;
    const pre = semver.prerelease(version);
    const supportedPrerelease = pre?.length === 2 && pre[0] === 'beta' && typeof pre[1] === 'number'
      || pre?.length === 1 && typeof pre[0] === 'string' && /^pre-release-[1-9]\d*$/.test(pre[0]);
    return track === 'stable' ? !release.prerelease && !pre : (!release.prerelease && !pre) || (release.prerelease && supportedPrerelease);
  }).sort((a, b) => semver.rcompare(a.tag_name, b.tag_name))[0];
}

async function githubReleases(): Promise<Release[]> {
  const releases: Release[] = [];
  // Follow pagination so a busy prerelease history cannot hide the stable candidate.
  for (let page = 1; ; page++) {
    const response = await fetch(`https://api.github.com/repos/${repository}/releases?per_page=100&page=${page}`, {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'Refract-updater' }, signal: AbortSignal.timeout(30_000)
    });
    if (!response.ok) throw new Error(`GitHub update check failed (${response.status}). Will retry in 10 minutes.`);
    const data: unknown = await response.json();
    if (!Array.isArray(data) || !data.every(r => r && typeof r.tag_name === 'string' && typeof r.draft === 'boolean' && typeof r.prerelease === 'boolean' && Array.isArray(r.assets) && r.assets.every((a: {name?: unknown}) => typeof a?.name === 'string'))) throw new Error('Invalid GitHub release response.');
    releases.push(...data as Release[]);
    if (!response.headers.get('link')?.includes('rel="next"')) return releases;
    if (page >= 100) throw new Error('GitHub release history is too large to check safely.');
  }
}

export interface UpdateClient {
  autoDownload: boolean; autoInstallOnAppQuit: boolean; allowPrerelease: boolean; allowDowngrade: boolean;
  checkForUpdates(): Promise<{updateInfo: {version: string}} | null>;
  downloadUpdate(): Promise<unknown>;
  quitAndInstall(isSilent: boolean, isForceRunAfter: boolean): void;
  on(event: 'error', listener: (error: Error) => void): unknown;
}
function createUpdateClient(release: Release): UpdateClient {
  const channel = semver.prerelease(release.tag_name) ? 'beta' : 'latest';
  if (!release.assets.some(a => a.name === `${channel}.yml`)) throw new Error(`Release ${release.tag_name} is missing update metadata. Will retry in 10 minutes.`);
  const { NsisUpdater }: typeof import('electron-updater') = require('electron-updater');
  const updater = new NsisUpdater({ provider: 'generic', url: `https://github.com/${repository}/releases/download/${encodeURIComponent(release.tag_name)}/`, channel });
  updater.autoInstallOnAppQuit = false;
  return updater;
}

class UpdateService {
  private track: UpdateTrack = 'stable';
  private state: UpdateStatus = {track: 'stable', phase: 'idle', message: 'Updates are checked every 10 minutes.'};
  private timer?: NodeJS.Timeout;
  private running?: Promise<void>;
  private generation = 0;
  private prepared?: UpdateClient;
  private preparedVersion?: string;
  private restarting = false;
  private pendingTrackChanges = 0;
  private saving: Promise<unknown> = Promise.resolve();
  constructor(private file: string, private version: string, private enabled: boolean,
    private releases = githubReleases, private client = createUpdateClient) {}
  status(): UpdateStatus { return {...this.state, track: this.track}; }
  async start(): Promise<void> {
    try { const data: unknown = JSON.parse(await fs.readFile(this.file, 'utf8')); if ((data as {track?: unknown})?.track === 'beta') this.track = 'beta'; }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') console.warn('Could not load update track; using stable.'); }
    if (!this.enabled) { this.state = {...this.status(), phase: 'disabled', message: 'Automatic updates are available in the installed Windows app.'}; return; }
    this.timer = setInterval(() => { void this.check(); }, CHECK_INTERVAL_MS); this.timer.unref();
    void this.check();
  }
  async setTrack(value: unknown): Promise<UpdateStatus> {
    if (this.restarting) throw new Error('An update restart is already in progress');
    if (value !== 'stable' && value !== 'beta') throw new Error('Invalid update track');
    this.pendingTrackChanges++;
    const save = this.saving.catch(() => {}).then(async () => {
      await fs.mkdir(path.dirname(this.file), {recursive: true});
      await fs.writeFile(this.file + '.tmp', JSON.stringify({track: value}) + '\n');
      await fs.rename(this.file + '.tmp', this.file);
      this.track = value; this.generation++;
      if (this.prepared) this.prepared.autoInstallOnAppQuit = false;
      this.prepared = undefined;
      if (this.running) await this.running;
      await this.check();
      return this.status();
    });
    this.saving = save;
    try { return await save; } finally { this.pendingTrackChanges--; }
  }
  check(): Promise<void> {
    if (!this.enabled || this.restarting) return Promise.resolve();
    if (this.running) return this.running;
    this.running = this.runCheck().finally(() => { this.running = undefined; }); return this.running;
  }
  private async runCheck(): Promise<void> {
    const generation = this.generation;
    const track = this.track;
    // Keep a downloaded update available while checking for a newer release.
    this.state = {track, phase: 'checking', message: 'Checking GitHub releases...'};
    let updater: UpdateClient | undefined;
    try {
      const release = selectRelease(await this.releases(), track, this.version);
      if (generation !== this.generation) return;
      if (!release) {
        if (this.prepared) this.prepared.autoInstallOnAppQuit = false;
        this.prepared = undefined;
        this.state = {track, phase: 'idle', message: 'You are up to date. Checking every 10 minutes.'}; return; }
      if (this.prepared && this.preparedVersion === release.tag_name) {
        this.state = {track, phase: 'ready', message: `${release.tag_name} is ready. Click Restart to update to install it.`}; return;
      }
      // A new download can replace the shared updater cache. Disarm the old installer first.
      if (this.prepared) this.prepared.autoInstallOnAppQuit = false;
      this.prepared = undefined;
      updater = this.client(release);
      updater.autoDownload = false; updater.autoInstallOnAppQuit = false;
      updater.allowPrerelease = track === 'beta'; updater.allowDowngrade = false;
      updater.on('error', error => console.error('Update error:', error.message)); // Check/download rejections also update Settings.
      const result = await updater.checkForUpdates();
      if (!result || !semver.eq(result.updateInfo.version, release.tag_name)) throw new Error('Release metadata does not match the selected version.');
      if (generation !== this.generation) return;
      this.state = {track, phase: 'downloading', message: `Downloading ${release.tag_name}...`};
      await updater.downloadUpdate();
      if (generation !== this.generation) return;
      if (this.prepared) this.prepared.autoInstallOnAppQuit = false;
      this.prepared = updater; this.preparedVersion = release.tag_name;
      this.state = {track, phase: 'ready', message: `${release.tag_name} is ready. Click Restart to update to install it.`};
    } catch (error) {
      if (updater) updater.autoInstallOnAppQuit = false;
      if (generation === this.generation) this.state = {track, phase: 'error', message: `${(error as Error).message} Updates will retry in 10 minutes.`};
    }
  }
  prepareRestart(): () => void {
    if (this.restarting || this.pendingTrackChanges > 0 || !this.prepared || this.state.phase !== 'ready' || this.running) throw new Error('No downloaded update is ready to install');
    const updater = this.prepared;
    this.restarting = true;
    this.stop();
    this.state = {track: this.track, phase: 'restarting', message: 'Saving your work and restarting to update...'};
    let installed = false;
    // The main process calls this only after its save/compiler/assistant shutdown barrier.
    // Show installer failures (locked files, permissions, or dependency setup)
    // instead of letting a silent installer disappear without an explanation.
    return () => { if (!installed) { installed = true; updater.quitAndInstall(false, true); } };
  }
  stop(): void { if (this.timer) clearInterval(this.timer); this.generation++; }
}

module.exports = { UpdateService, selectRelease, githubReleases, createUpdateClient, CHECK_INTERVAL_MS };
