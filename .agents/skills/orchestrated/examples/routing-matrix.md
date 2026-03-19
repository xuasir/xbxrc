# Routing Matrix

| Situation | Route |
|---|---|
| Single obvious local bug | Coding -> Verifier |
| Bug with unclear root cause | Reader -> Analysis -> Coding -> Verifier |
| Multi-file feature | Reader -> Analysis (if needed) -> Coding -> Verifier |
| Large refactor | Reader -> Analysis -> Coding (atomic patches) -> Verifier |
| Performance hot path | Reader -> Analysis -> Coding -> Verifier |
| Need crisp acceptance contract first | Spec -> Reader/Analysis -> Coding -> Verifier |

## Model Preference
- Reader: low-cost model
- Analysis: 5.4-mini high reasoning
- Coding: 5.3-codex medium reasoning
- Verifier: practical validator model/tool-backed if available
