export type Phase = 'queue' | 'prepare' | 'snapshotBefore' | 'context' | 'providerStartup' | 'providerTurn' | 'tool' | 'build' | 'snapshotAfter';
export interface PerformanceReport { totalMs: number; phasesMs: Partial<Record<Phase, number>>; counts: Record<string, number> }
class AssistantPerformance {
  private readonly started = performance.now();
  private readonly phases: Partial<Record<Phase, number>> = {};
  private readonly counts: Record<string, number> = {};
  add(phase: Phase, milliseconds: number) { this.phases[phase] = (this.phases[phase] || 0) + Math.max(0,milliseconds); }
  count(name: 'toolCalls' | 'builds' | 'buildCacheHits' | 'providerReuses' | 'contextCharacters', amount = 1) { this.counts[name] = (this.counts[name] || 0) + amount; }
  async measure<T>(phase: Phase, work: () => Promise<T>): Promise<T> {
    const start=performance.now(); try {return await work();} finally {this.add(phase,performance.now()-start);}
  }
  report(): PerformanceReport { return {totalMs:performance.now()-this.started,phasesMs:{...this.phases},counts:{...this.counts}}; }
}
export = {AssistantPerformance};
