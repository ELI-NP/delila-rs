import { Injectable, signal, computed, effect, inject, untracked } from '@angular/core';
import { OperatorService } from './operator.service';
import { TimerExpiredNotice } from '../models/types';

/** Longest run timer the Operator accepts (TODO 86): 24 h. */
export const MAX_TIMER_MINUTES = 24 * 60;

/** "1:05" / "2:03:04" */
export function formatSeconds(total: number): string {
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`;
  }
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

/**
 * The run timer as the pages show it (TODO 86).
 *
 * The Operator owns the timer: it gets the length with Start (or
 * PUT /api/run/timer) and stops the run itself, like the Stop button, whether
 * or not a browser is open. This service only
 * - counts down between status polls from the Operator's `remaining_ms`
 *   (so a hidden, throttled tab or a wrong PC clock does not matter: every
 *   poll puts it right), for the header and the Control Panel, and
 * - rings the alarm once on every open page when the Operator reports that the
 *   time is up (`timer_expired`), except for an expiry that was already there
 *   when the page opened.
 */
@Injectable({
  providedIn: 'root',
})
export class TimerService {
  private readonly operator = inject(OperatorService);

  // Form values for the next Start, or for "Set timer" during a run
  readonly durationMinutes = signal(120);
  readonly autoStop = signal(true);

  /** performance.now(), moved once a second so the countdown moves between polls. */
  readonly now = signal(performance.now());

  /** The current run's timer, as the Operator last reported it. */
  readonly current = computed(() => this.operator.runInfo()?.timer ?? null);
  readonly isRunning = computed(() => this.current() !== null);

  /** Milliseconds left, negative once over (alarm-only timers). */
  readonly remainingMs = computed(() => {
    const t = this.current();
    if (!t) return 0;
    const sinceStatus = Math.max(0, this.now() - this.operator.statusReceivedAt());
    return t.remaining_ms - sinceStatus;
  });

  /** "1:23:45 left" / "+3:12 over" / "" without a timer. */
  readonly headerText = computed(() => {
    if (!this.current()) return '';
    const ms = this.remainingMs();
    return ms >= 0
      ? `${formatSeconds(Math.ceil(ms / 1000))} left`
      : `+${formatSeconds(Math.floor(-ms / 1000))} over`;
  });

  readonly progress = computed(() => {
    const t = this.current();
    if (!t) return 0;
    const total = t.minutes * 60_000;
    return Math.min(100, Math.max(0, ((total - this.remainingMs()) / total) * 100));
  });

  /** The expiry to alarm for, until dismissed. */
  readonly alarm = signal<TimerExpiredNotice | null>(null);
  private baselineTaken = false;
  private lastSeenAt: number | null = null;

  private audioContext: AudioContext | null = null;
  private alarmIntervalId: ReturnType<typeof setInterval> | null = null;

  constructor() {
    setInterval(() => this.now.set(performance.now()), 1000);
    effect(() => {
      const status = this.operator.status();
      if (!status) return;
      const notice = status.timer_expired ?? null;
      untracked(() => this.observe(notice));
    });
    // Browsers play sound only after the page had a user gesture: prepare the
    // audio on the first click so the alarm can sound later on this page.
    document.addEventListener('pointerdown', () => this.ensureAudioContext(), {
      once: true,
      capture: true,
    });
  }

  /** Feed the expiry notice of one status. The first status of the page only
   *  records what was already there. */
  observe(notice: TimerExpiredNotice | null): void {
    const at = notice?.at_ms ?? null;
    if (!this.baselineTaken) {
      this.baselineTaken = true;
      this.lastSeenAt = at;
      return;
    }
    if (notice && at !== this.lastSeenAt) {
      this.lastSeenAt = at;
      this.alarm.set(notice);
    }
  }

  dismissAlarm(): void {
    this.alarm.set(null);
    this.stopAlarm();
  }

  // Start continuous alarm sound
  startAlarm(): void {
    if (this.alarmIntervalId) return;
    this.playBeep();
    // Continue playing every 2 seconds
    this.alarmIntervalId = setInterval(() => {
      this.playBeep();
    }, 2000);
  }

  // Stop alarm sound
  stopAlarm(): void {
    if (this.alarmIntervalId) {
      clearInterval(this.alarmIntervalId);
      this.alarmIntervalId = null;
    }
    // Suspend but don't close — reuse for the next alarm
    if (this.audioContext && this.audioContext.state === 'running') {
      this.audioContext.suspend();
    }
  }

  /** Create or resume the AudioContext. Works only after a user gesture on the page. */
  private ensureAudioContext(): void {
    if (!this.audioContext || this.audioContext.state === 'closed') {
      this.audioContext = new AudioContext();
    }
    if (this.audioContext.state === 'suspended') {
      this.audioContext.resume();
    }
  }

  private playBeep(): void {
    try {
      this.ensureAudioContext();
      const ctx = this.audioContext!;

      // Play a series of beeps
      for (let i = 0; i < 3; i++) {
        const oscillator = ctx.createOscillator();
        const gainNode = ctx.createGain();

        oscillator.connect(gainNode);
        gainNode.connect(ctx.destination);

        oscillator.frequency.value = 880; // A5 note
        oscillator.type = 'sine';

        const startTime = ctx.currentTime + i * 0.3;
        gainNode.gain.setValueAtTime(0.5, startTime);
        gainNode.gain.exponentialRampToValueAtTime(0.01, startTime + 0.2);

        oscillator.start(startTime);
        oscillator.stop(startTime + 0.2);
      }
    } catch (e) {
      console.error('Failed to play alarm:', e);
    }
  }
}
