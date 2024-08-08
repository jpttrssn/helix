use helix_view::Document;
use std::io::Read;

use super::*;

fn assert_gopls(doc: Option<&Document>) {
    assert!(doc.is_some(), "doc not found");
    if let Some(doc) = doc {
        assert!(
            doc.language_servers()
                .find(|s| s.name() == "gopls")
                .is_some(),
            "gopls language server not found"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_organize_imports_go() -> anyhow::Result<()> {
    let lang_conf = indoc! {r#"
            [[language]]
            name = "go"
            code-actions-on-save = [{ code-action = "source.organizeImports", enabled = true }]
            indent = { tab-width = 4, unit = " " }
        "#};

    let text = indoc! {r#"
            #[p|]#ackage main
        
            import "fmt"

            import "path"

            func main() {
             fmt.Println("a")
                path.Join("b")
            }
        "#};

    let mut file = tempfile::Builder::new().suffix(".go").tempfile()?;
    let mut app = helpers::AppBuilder::new()
        .with_config(Config::default())
        .with_lang_loader(helpers::test_syntax_loader(Some(lang_conf.into())))
        .with_file(file.path(), None)
        .with_input_text(text)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            // Check that we have gopls available while also allowing
            // for gopls to initialize before sending key sequences
            (
                None,
                Some(&|app| {
                    let doc = app.editor.document_by_path(file.path());
                    assert_gopls(doc);
                }),
            ),
            (Some(":w<ret>"), None),
        ],
        false,
    )
    .await?;

    reload_file(&mut file).unwrap();
    let mut file_content = String::new();
    file.as_file_mut().read_to_string(&mut file_content)?;

    assert_eq!(
        LineFeedHandling::Native.apply("package main\n\nimport (\n\t\"fmt\"\n\t\"path\"\n)\n\nfunc main() {\n\tfmt.Println(\"a\")\n\tpath.Join(\"b\")\n}\n"),
        file_content
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_organize_imports_go_multi() -> anyhow::Result<()> {
    let lang_conf = indoc! {r#"
            [[language]]
            name = "go"
            code-actions-on-save = [{ code-action = "source.organizeImports", enabled = true }]
        "#};

    let text = indoc! {r#"
            #[p|]#ackage main
        
            import "path"
            import "fmt"

            func main() {
             fmt.Println("a")
                path.Join("b")
            }
        "#};

    let mut file1 = tempfile::Builder::new().suffix(".go").tempfile()?;
    let mut file2 = tempfile::Builder::new().suffix(".go").tempfile()?;
    let mut app = helpers::AppBuilder::new()
        .with_config(Config::default())
        .with_lang_loader(helpers::test_syntax_loader(Some(lang_conf.into())))
        .with_file(file1.path(), None)
        .with_input_text(text)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some(&format!(
                    ":o {}<ret>ipackage main<ret>import \"fmt\"<ret>func test()<ret><esc>",
                    file2.path().to_string_lossy(),
                )),
                None,
            ),
            // Check that we have gopls available while also allowing
            // for gopls to initialize before sending key sequences
            (
                None,
                Some(&|app| {
                    let doc1 = app.editor.document_by_path(file1.path());
                    assert_gopls(doc1);
                    let doc2 = app.editor.document_by_path(file2.path());
                    assert_gopls(doc2);
                }),
            ),
            (Some(":wa<ret>"), None),
        ],
        false,
    )
    .await?;

    reload_file(&mut file1).unwrap();
    let mut file_content1 = String::new();
    file1.as_file_mut().read_to_string(&mut file_content1)?;

    assert_eq!(
        LineFeedHandling::Native.apply("package main\n\nimport (\n\t\"fmt\"\n\t\"path\"\n)\n\nfunc main() {\n\tfmt.Println(\"a\")\n\tpath.Join(\"b\")\n}\n"),
        file_content1
    );

    reload_file(&mut file2).unwrap();
    let mut file_content2 = String::new();
    file2.as_file_mut().read_to_string(&mut file_content2)?;

    assert_eq!(
        LineFeedHandling::Native.apply("package main\n\nfunc test()\n"),
        file_content2
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invalid_code_action_go() -> anyhow::Result<()> {
    let lang_conf = indoc! {r#"
            [[language]]
            name = "go"
            code-actions-on-save = [{ code-action = "source.invalid", enabled = true }]
        "#};

    let text = indoc! {r#"
            #[p|]#ackage main
        
            import "fmt"

            import "path"

            func main() {
                fmt.Println("a")
                path.Join("b")
            }
        "#};

    let mut file = tempfile::Builder::new().suffix(".go").tempfile()?;
    let mut app = helpers::AppBuilder::new()
        .with_config(Config::default())
        .with_lang_loader(helpers::test_syntax_loader(Some(lang_conf.into())))
        .with_file(file.path(), None)
        .with_input_text(text)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            // Check that we have gopls available while also allowing
            // for gopls to initialize before sending key sequences
            (
                None,
                Some(&|app| {
                    let doc = app.editor.document_by_path(file.path());
                    assert_gopls(doc);
                }),
            ),
            (
                Some(":w<ret>"),
                Some(&|app| {
                    assert!(!app.editor.is_err(), "error: {:?}", app.editor.get_status());
                }),
            ),
        ],
        false,
    )
    .await?;

    reload_file(&mut file).unwrap();
    let mut file_content = String::new();
    file.as_file_mut().read_to_string(&mut file_content)?;

    assert_eq!(
        LineFeedHandling::Native.apply("package main\n\nimport \"fmt\"\n\nimport \"path\"\n\nfunc main() {\n\tfmt.Println(\"a\")\n\tpath.Join(\"b\")\n}\n"),
        file_content
    );

    Ok(())
}
