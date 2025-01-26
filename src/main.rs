extern crate spider;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::HashMap;
use spider::percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use spider::url::Url;
use spider::website::Website;
use spider::{reqwest, tokio, CaseInsensitiveString};
use spider_utils::spider_transformations::transform_content;
use spider_utils::spider_transformations::transformation::content::{
    ReturnFormat, TransformConfig,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
type Cache = Mutex<HashMap<CaseInsensitiveString, String>>;

async fn create_sitemap(cache: web::Data<Cache>) -> impl Responder {
    let mut website: Website = Website::new("https://www.heygoody.com/");
    website
        .configuration
        .with_respect_robots_txt(true)
        .with_user_agent(Some("SpiderBot"))
        .with_ignore_sitemap(false)
        .with_sitemap(Some("/sitemap/sitemap-0.xml"))
        .with_caching(true);
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
    let duration = start.elapsed();

    let mut response_body = String::from("Sitemap created successfully.\n\n");
    response_body.push_str(&format!(
        "Time elapsed: {:?} seconds for total pages: {}\n\n",
        duration.as_secs_f64(),
        links.len()
    ));

    let mut cache_lock = cache.lock().unwrap();
    for link in links.iter() {
        response_body.push_str(&format!("- {}\n", link.as_ref()));
        cache_lock.insert(link.to_string().into(), "Crawled".to_string());
    }

    HttpResponse::Ok().body(response_body)
}

//detect spa
async fn detect_spa(url: &str) -> bool {
    let response = reqwest::get(url).await.unwrap();
    let body = response.text().await.unwrap();
    //detect html content
    body.contains("<script>") || body.contains("fetch(") || body.contains("window.onload")
}

async fn html_to_markdown(cache: web::Data<Cache>) -> impl Responder {
    println!("create target/downloads directory");
    std::fs::create_dir_all("./target/downloads").unwrap_or_default();

    let mut website: Website = Website::new("https://www.heygoody.com/");
    website.configuration.with_caching(true);

    println!("Starting crawl_sitemap...");
    website.crawl_sitemap().await;
    website.persist_links();
    website.crawl().await;
    println!("Finished crawl_sitemap.");

    let links = website.get_all_links_visited().await;
    let mut conf = TransformConfig::default();
    conf.return_format = ReturnFormat::Markdown;

    // cache
    let mut cache_lock = cache.lock().unwrap();

    for link in links.iter() {
        let link_ci = CaseInsensitiveString::from(link.as_ref());

        if cache_lock.contains_key(&link_ci) {
            println!("Skipping cached link: {}", link);
            continue;
        }
        cache_lock.insert(link_ci, "Crawled".to_string());

        let is_spa = detect_spa(link.as_ref()).await;

        let mut web = if is_spa {
            let mut web_spa = Website::new(link.as_ref());
            let parsed_url = Url::parse(link.as_ref()).ok().map(Box::new);
            web_spa.configuration.with_chrome_intercept(RequestInterceptConfiguration::new(true), &parsed_url);
            web_spa
        } else {
            let mut web_ssr = Website::new(link.as_ref());
            web_ssr.configuration.with_user_agent(Some("SpiderBot"));
            web_ssr
        };

        let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> =
            web.subscribe(0).unwrap();

        let mut stdout: tokio::io::Stdout = tokio::io::stdout();

        // download file
        let download_file = percent_encode(link.as_ref().as_bytes(), NON_ALPHANUMERIC).to_string();
        let download_file = format!("./target/downloads/{}.md", download_file);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&download_file)
            .expect("Unable to open file");

        let join_handle = tokio::spawn(async move {
            while let Ok(res) = rx2.recv().await {
                let markup = transform_content(&res, &conf, &None, &None, &None);
                file.write_all(format!("- {}\n{}\n", res.get_url(), markup).as_bytes())
                    .unwrap_or_default();
                let _ = stdout
                    .write_all(format!("- {}\n {}\n", res.get_url(), markup).as_bytes())
                    .await;
            }
            stdout
        });

        let start = std::time::Instant::now();

        // เลือกวิธีการ crawl ที่เหมาะสม
        web.crawl().await;

        let duration = start.elapsed();
        let mut stdout = join_handle.await.unwrap();

        let _ = stdout
            .write_all(
                format!(
                    "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
                    duration,
                    web.get_size().await
                )
                .as_bytes(),
            )
            .await;
    }

    HttpResponse::Ok().body("HTML to Markdown conversion completed successfully.")
}




#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let cache: Cache = Mutex::new(HashMap::new());
    let cache_data = web::Data::new(cache);

    HttpServer::new(move || {
        App::new()
            .app_data(cache_data.clone()) // เพิ่ม Cache เข้าไปในแอป
            .route("/create-sitemap", web::get().to(create_sitemap))
            .route("/html_to_markdown", web::get().to(html_to_markdown))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
