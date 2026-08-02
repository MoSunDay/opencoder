    use super::*;

    fn begin(id: &str, fmt: &str, total: usize) -> Frame {
        Frame::Begin {
            id: id.into(),
            fmt: fmt.into(),
            total,
        }
    }
    fn chunk(id: &str, seq: usize, data: &str) -> Frame {
        Frame::Chunk {
            id: id.into(),
            seq,
            data: data.into(),
        }
    }
    fn feed_pending(asm: &mut Assembly, lines: &[&str]) {
        for line in lines {
            assert_eq!(asm.feed_line(line, 0), FeedOutcome::Pending, "line: {line}");
        }
    }
    fn feed_warn_contains(asm: &mut Assembly, line: &str, want: &str) {
        match asm.feed_line(line, 0) {
            FeedOutcome::Warn { message } => assert!(message.contains(want), "{message}"),
            other => panic!("expected Warn containing {want:?}, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_valid_variants() {
        assert_eq!(
            parse_frame("ocimg begin abc png 3"),
            Some(begin("abc", "png", 3))
        );
        assert_eq!(
            parse_frame("ocimg chunk abc 0 AAAA"),
            Some(chunk("abc", 0, "AAAA"))
        );
        assert_eq!(
            parse_frame("ocimg end abc"),
            Some(Frame::End { id: "abc".into() })
        );
        // leading/trailing whitespace tolerated
        assert_eq!(
            parse_frame("  ocimg begin abc png 3\t"),
            Some(begin("abc", "png", 3))
        );
    }

    #[test]
    fn parse_frame_rejects_malformed() {
        assert_eq!(parse_frame("ocimg start abc png 3"), None);
        assert_eq!(parse_frame("ocimg begin abc png"), None);
        assert_eq!(parse_frame("ocimg chunk abc 0"), None);
        assert_eq!(parse_frame("ocimg chunk abc zero AAAA"), None);
        assert_eq!(parse_frame("ocimg begin abc png x"), None);
        assert_eq!(parse_frame("ocimg begin abc png 3 extra"), None);
        assert_eq!(parse_frame("ocimg end abc extra"), None);
        assert_eq!(parse_frame("hello"), None);
        assert_eq!(parse_frame(""), None);
        assert_eq!(parse_frame("ocimg"), None);
    }

    #[test]
    fn whole_block_in_one_feed_completes() {
        let mut asm = Assembly::new();
        let lines = [
            "ocimg begin img1 png 3",
            "ocimg chunk img1 0 AAA",
            "ocimg chunk img1 1 BBB",
            "ocimg chunk img1 2 CCC",
        ];
        feed_pending(&mut asm, &lines);
        assert_eq!(
            asm.feed_line("ocimg end img1", 0),
            FeedOutcome::Complete {
                uri: "data:image/png;base64,AAABBBCCC".to_string(),
                filename: "pasted.png".to_string(),
                chunks: 3,
            }
        );
        assert!(asm.is_empty());
    }

    #[test]
    fn out_of_order_seqs_concat_in_order() {
        let mut asm = Assembly::new();
        let lines = [
            "ocimg begin x png 3",
            "ocimg chunk x 2 CCC",
            "ocimg chunk x 0 AAA",
            "ocimg chunk x 1 BBB",
        ];
        feed_pending(&mut asm, &lines);
        match asm.feed_line("ocimg end x", 0) {
            FeedOutcome::Complete { uri, .. } => assert_eq!(uri, "data:image/png;base64,AAABBBCCC"),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_chunk_overwrites() {
        let mut asm = Assembly::new();
        let lines = [
            "ocimg begin x png 2",
            "ocimg chunk x 0 AAA",
            "ocimg chunk x 1 OLD",
        ];
        feed_pending(&mut asm, &lines);
        assert_eq!(
            asm.feed_line("ocimg chunk x 1 BBB", 0),
            FeedOutcome::Pending
        );
        match asm.feed_line("ocimg end x", 0) {
            FeedOutcome::Complete { uri, .. } => assert_eq!(uri, "data:image/png;base64,AAABBB"),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn end_with_missing_chunk_warns_and_drops() {
        let mut asm = Assembly::new();
        feed_pending(&mut asm, &["ocimg begin x png 2", "ocimg chunk x 0 AAA"]);
        feed_warn_contains(&mut asm, "ocimg end x", "1/2");
        assert_eq!(asm.in_flight(), 0);
    }

    #[test]
    fn chunk_for_unknown_id_warns() {
        feed_warn_contains(&mut Assembly::new(), "ocimg chunk nope 0 AAA", "unknown id");
    }

    #[test]
    fn end_for_unknown_id_warns() {
        feed_warn_contains(&mut Assembly::new(), "ocimg end nope", "unknown id");
    }

    #[test]
    fn duplicate_begin_warns_and_restarts() {
        let mut asm = Assembly::new();
        feed_pending(&mut asm, &["ocimg begin x png 2", "ocimg chunk x 0 AAA"]);
        feed_warn_contains(&mut asm, "ocimg begin x png 1", "duplicate begin");
        // Old chunks are gone: only the new single chunk completes.
        asm.feed_line("ocimg chunk x 0 ZZZ", 0);
        match asm.feed_line("ocimg end x", 0) {
            FeedOutcome::Complete { uri, chunks, .. } => {
                assert_eq!(uri, "data:image/png;base64,ZZZ");
                assert_eq!(chunks, 1);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn seq_out_of_range_warns() {
        let mut asm = Assembly::new();
        asm.feed_line("ocimg begin x png 1", 0);
        feed_warn_contains(&mut asm, "ocimg chunk x 1 AAA", "out of range");
    }

    #[test]
    fn unknown_fmt_warns() {
        let mut asm = Assembly::new();
        feed_warn_contains(&mut asm, "ocimg begin x tiff 1", "unknown image format");
        assert!(asm.is_empty());
    }

    #[test]
    fn zero_total_warns() {
        let mut asm = Assembly::new();
        assert!(matches!(
            asm.feed_line("ocimg begin x png 0", 0),
            FeedOutcome::Warn { .. }
        ));
        assert!(asm.is_empty());
    }

    #[test]
    fn jpeg_fmt_maps_mime() {
        let mut asm = Assembly::new();
        feed_pending(&mut asm, &["ocimg begin x jpg 1", "ocimg chunk x 0 AAA"]);
        match asm.feed_line("ocimg end x", 0) {
            FeedOutcome::Complete { uri, filename, .. } => {
                assert!(uri.starts_with("data:image/jpeg;base64,"));
                assert_eq!(filename, "pasted.jpeg");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn non_frame_line_is_not_frame() {
        let mut asm = Assembly::new();
        asm.feed_line("ocimg begin x png 1", 0);
        assert_eq!(asm.feed_line("hello", 0), FeedOutcome::NotFrame);
        assert_eq!(asm.in_flight(), 1);
    }

    #[test]
    fn drain_stale_drops_old_entries() {
        let mut asm = Assembly::new();
        asm.feed_line("ocimg begin old png 2", 1_000);
        let drained = asm.drain_stale(1_000 + STALE_TIMEOUT_MS, STALE_TIMEOUT_MS);
        assert_eq!(drained, vec![("old".to_string(), 0, 2)]);
        assert!(asm.is_empty());

        // A fresh begin is not drained by a small clock advance.
        asm.feed_line("ocimg begin fresh png 2", 5_000);
        assert!(asm.drain_stale(5_000 + 1_000, STALE_TIMEOUT_MS).is_empty());
        assert_eq!(asm.in_flight(), 1);
    }
