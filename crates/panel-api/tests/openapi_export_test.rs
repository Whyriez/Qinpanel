use std::fs;
use std::path::PathBuf;
use utoipa::OpenApi;

#[test]
fn managed_storage_request_id_has_string_schema() {
    let doc = serde_json::to_value(panel_api::routes::ApiDoc::openapi())
        .expect("OpenAPI specification should serialize");
    assert_eq!(
        doc["components"]["schemas"]["ManagedStorageAllocation"]["properties"]["request_id"]["type"],
        "string"
    );
}

#[test]
fn test_export_openapi_specification() {
    let doc = panel_api::routes::ApiDoc::openapi();

    let json_str = serde_json::to_string_pretty(&doc)
        .expect("ApiDoc openapi specification should serialize to pretty JSON");

    assert!(
        json_str.contains("Qinpanel API"),
        "OpenAPI spec must contain Qinpanel API title"
    );
    assert!(
        json_str.contains("/api/v1/system/info"),
        "OpenAPI spec must contain system info path"
    );

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("Workspace root should exist");
    let docs_dir = workspace_root.join("docs");
    let openapi_path = docs_dir.join("openapi.json");

    fs::create_dir_all(&docs_dir).expect("docs directory should exist or be created");
    fs::write(&openapi_path, &json_str).expect("docs/openapi.json should be written successfully");

    assert!(
        openapi_path.exists(),
        "Exported docs/openapi.json file must exist"
    );
    let read_back = fs::read_to_string(&openapi_path).expect("File must be readable");
    assert_eq!(read_back, json_str);
}
