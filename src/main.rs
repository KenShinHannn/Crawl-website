extern crate spider;
use spider::website::Website;
use spider::{tokio};
use spider_utils::spider_transformations::transform_content;
use spider_utils::spider_transformations::transformation::content::{
    ReturnFormat, TransformConfig,
};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    
    let mut website: Website = Website::new("https://www.heygoody.com/");
    println!("website: {:?}", website);

    website
        .configuration
        .with_respect_robots_txt(true)
        .with_user_agent(Some("SpiderBot"))
        .with_ignore_sitemap(true) // ignore running the sitemap on base crawl/scape methods. Remove or set to true to include the sitemap with the crawl.
        .with_sitemap(Some("/sitemap/sitemap-0.xml"));

    let start = Instant::now();

    // crawl the sitemap first
    println!("Starting crawl_sitemap...");
    website.crawl_sitemap().await;
    println!("Finished crawl_sitemap.");
    // persist links to the next crawl
    println!("Starting persist_links...");
    website.persist_links();
    println!("Finished persist_links.");
    // crawl normal with links found in the sitemap extended.
    println!("Starting crawl...");
    website.crawl().await;
    println!("Finished crawl.");

    let links = website.get_all_links_visited().await;

    let mut conf = TransformConfig::default();
    conf.return_format = ReturnFormat::Markdown;

    for link in links.iter() {
        let mut web: Website = Website::new(link.as_ref());
        let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> = 
        web.subscribe(0).unwrap();
        let mut stdout: tokio::io::Stdout = tokio::io::stdout();

        let join_handle = tokio::spawn(async move {
            while let Ok(res) = rx2.recv().await {
                let markup = transform_content(&res, &conf, &None, &None, &None);
    
                let _ = stdout
                    .write_all(format!("- {}\n {}\n", res.get_url(), markup).as_bytes())
                    .await;
            }
            stdout
        });
        let start = std::time::Instant::now();
        web.crawl_smart().await;
        let duration = start.elapsed();
        let mut stdout = join_handle.await.unwrap();
    
        let _ = stdout
            .write_all(
                format!(
                    "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
                    duration,
                    website.get_size().await
                )
                .as_bytes(),
            )
            .await;
    }

    let duration: std::time::Duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}

