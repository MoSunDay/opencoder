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

## 本轮评审/交付/收尾报告(live-region-steps-only-say-in-transcript,3e609c7ef)

- [review-live-region-steps-only-say-in-transcript.md](review-live-region-steps-only-say-in-transcript.md)
- [delivery-live-region-steps-only-say-in-transcript.md](delivery-live-region-steps-only-say-in-transcript.md)
- [final-live-region-steps-only-say-in-transcript.md](final-live-region-steps-only-say-in-transcript.md)

**为什么现在可上线**:需求(实时区 steps-only、无 Say、turn 终止清空、Say
入转录共享滚动、显示 bug 修复)全部实现;定向门禁全绿(见 delivery 门禁
事实);二进制与 server 已部署并验证(见 final 线上验证清单);用户指示
完成收尾。
