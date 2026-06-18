# OKRs

We use **Objectives and Key Results (OKRs)** to set direction and measure progress. OKRs complement
our [engineering practices](engineering-practices.md): the practices are *how* we work; OKRs are
*what outcomes* we're aiming for this quarter.

## How we use OKRs

- **Objective** — a qualitative, ambitious, memorable statement of *what* we want to achieve. No
  numbers in the objective itself.
- **Key Results** — **3–5** measurable outcomes per objective that prove the objective is met. Each
  KR is a number with a baseline and a target. KRs measure *outcomes*, not activity (not "ship X"
  but "X moves metric Y from A to B").
- **Cadence** — set quarterly, reviewed regularly (e.g. weekly check-ins, mid-quarter adjustment).
- **Ambition** — OKRs are stretch goals. We aim for roughly **70%** attainment. Consistently
  hitting 100% means the OKRs were set too conservatively; consistently low means too aggressive.
- **Grading** — at quarter end, grade each KR (commonly 0.0–1.0). The grade informs the next
  quarter; it is not a performance-review weapon.

## Template

```
Objective: <qualitative, ambitious statement of intent>

  KR1: <metric> from <baseline> to <target>   (owner)
  KR2: <metric> from <baseline> to <target>   (owner)
  KR3: <metric> from <baseline> to <target>   (owner)
  [KR4 / KR5 optional]

Confidence: <low | medium | high>   (updated at each check-in)
```

## Illustrative examples

> These examples are **illustrative**, not committed targets. They show the *shape* of good OKRs
> grounded in this product; real baselines and targets are set each quarter against measured data.

### Example 1 — Make reviews genuinely useful (illustrative)

```
Objective: Lightbridge reviews are trusted enough that engineers act on them.

  KR1: Share of Lightbridge findings marked helpful by reviewers  from 45% to 70%
  KR2: False-positive rate of posted findings                     from 30% to 12%
  KR3: PRs where a Lightbridge finding led to a code change        from 10% to 25%
  KR4: Median time from `@lightbridge` mention to posted review    from 6 min to 3 min
```

### Example 2 — Faster onboarding and indexing (illustrative)

```
Objective: New repositories become useful quickly and stay fresh.

  KR1: Median baseline indexing latency for a mid-size repo  from 18 min to 8 min
  KR2: Time-to-first-useful-answer for a newly onboarded repo from 25 min to 10 min
  KR3: Share of PR reviews served from an incremental overlay  from 40% to 80%
  KR4: Stale-index incidents per month                         from 6 to 1
```

## Notes

- Tie KRs to metrics we actually emit (see
  [observability](../security-observability-testing-rollout.md#observability)) so progress is
  measured, not guessed.
- Keep the set small: a team typically carries 1–2 objectives at a time, not five.
