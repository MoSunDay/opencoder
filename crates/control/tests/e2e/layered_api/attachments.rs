use super::*;

#[tokio::test]
async fn image_attachments_roundtrip_and_reject_invalid_uploads() {
    let h = Harness::with_brain_kind().await;
    let data = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let (status, reference) = h
        .req(
            Method::POST,
            "/api/brain/attachments",
            Some(json!({"name":"sample.png","data_url":data})),
        )
        .await;
    assert_eq!(status, 200, "{reference}");
    assert_eq!(reference["mime"], "image/png");
    assert_eq!(reference["sha256"].as_str().unwrap().len(), 64);
    let (status, stored) = h
        .req(
            Method::GET,
            &format!(
                "/api/brain/attachments/{}",
                reference["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, 200, "{stored}");
    assert_eq!(stored["reference"], reference);
    assert_eq!(stored["data_url"], data);
    for upload in [
        json!({"name":"bad.svg","data_url":"data:image/svg+xml;base64,PHN2Zz4="}),
        json!({"name":"bad.png","data_url":"data:image/png;base64,aGVsbG8="}),
        json!({"name":"","data_url":data}),
    ] {
        let (status, rejection) = h
            .req(Method::POST, "/api/brain/attachments", Some(upload))
            .await;
        assert_eq!(status, 400, "{rejection}");
    }
    let (status, _) = h
        .req(Method::GET, "/api/brain/attachments/image-missing", None)
        .await;
    assert_eq!(status, 404);
}
