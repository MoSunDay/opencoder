# 2026-11-08 变更记录索引

## live region 演进链(按时间序,后者取代前者)

1. [live-region-below-say-above-tok-cost.md](live-region-below-say-above-tok-cost.md)
2. [live-region-calls-expanded-input.md](live-region-calls-expanded-input.md)(已取代)
3. [live-region-hugs-content-say-deferred.md](live-region-hugs-content-say-deferred.md)(已取代)
4. [live-region-calls-match-ladder.md](live-region-calls-match-ladder.md)
5. [live-region-steps-only-say-in-transcript.md](live-region-steps-only-say-in-transcript.md)(已取代,Say 入转录共享滚动仍现行)
6. [live-region-one-scrollbar-folded-say-ladder.md](live-region-one-scrollbar-folded-say-ladder.md)(现行口径)

## 独立条目

- [compaction-tail-zero-default.md](compaction-tail-zero-default.md)
- [turso-write-lock-contention-hardening.md](turso-write-lock-contention-hardening.md)

## 评审/收尾报告

### live-region-one-scrollbar-folded-say-ladder(b3ad08ad9,现行口径)

- [review-live-region-one-scrollbar-folded-say-ladder.md](review-live-region-one-scrollbar-folded-say-ladder.md)
- [final-live-region-one-scrollbar-folded-say-ladder.md](final-live-region-one-scrollbar-folded-say-ladder.md)

**为什么现在可上线**:需求(live 并入转录滚动窗共一条滚动条、流式期阶梯
折叠 + Say 独立渲染、seal 后才合并)全部实现;定向门禁全绿(见条目内定向
回归段);二进制已部署(b3ad08ad9),oc-server active/HTTP 200、SPA 无漂移;
用户重启 TUI 核验渲染正常,并确认 web 交互逻辑对齐。

### live-region-steps-only-say-in-transcript(3e609c7ef,已取代)

- [review-live-region-steps-only-say-in-transcript.md](review-live-region-steps-only-say-in-transcript.md)
- [delivery-live-region-steps-only-say-in-transcript.md](delivery-live-region-steps-only-say-in-transcript.md)
- [final-live-region-steps-only-say-in-transcript.md](final-live-region-steps-only-say-in-transcript.md)
