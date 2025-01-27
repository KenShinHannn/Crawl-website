extern crate spider;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::HashMap;
use spider::percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use spider::url::Url;
use spider::website::Website;
use spider::{ reqwest, tokio, CaseInsensitiveString};
use spider_utils::spider_transformations::transform_content;
use spider_utils::spider_transformations::transformation::content::{
    ReturnFormat, TransformConfig,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;
use tokio::io::AsyncWriteExt;

type Cache = Mutex<HashMap<String, String>>;

pub async fn create_sitemap(cache: web::Data<Cache>) -> (HttpResponse, Vec<String>) {
    let url: &str = "https://www.heygoody.com/robots.txt";
    let web_url = "https://www.heygoody.com";

    let is_spa = detect_spa(web_url.as_ref()).await;

    let mut website: Website = if is_spa {
        let mut web_spa = Website::new(web_url.as_ref());
        let parsed_url = Url::parse(web_url.as_ref()).ok().map(Box::new);
        web_spa.configuration
            .with_respect_robots_txt(true)  
            .with_chrome_intercept(RequestInterceptConfiguration::new(true), &parsed_url)
            .with_caching(true);
        web_spa
    } else {
        let mut web_ssr = Website::new(web_url.as_ref());
        web_ssr.configuration
            .with_respect_robots_txt(true)
            .with_user_agent(Some("SpiderBot")) 
            .with_caching(true);
        web_ssr
    };
    let start: Instant = Instant::now();

    let response = reqwest::get(url).await.unwrap();
    let body = response.text().await.unwrap();

    let sitemaps: Vec<String> = body.lines()
        .filter(|line| line.starts_with("Sitemap:"))
        .map(|line| line.trim_start_matches("Sitemap:").trim().to_string())
        .collect();
    println!("sitemap {:?}", sitemaps);

    let mut total_pages = 0;
    let mut extracted_links: Vec<String> = Vec::new();
    let mut cache_lock = cache.lock().unwrap();

    for link in &sitemaps {
        let sitemap_url = Url::parse(link).unwrap();
        let sitemap_path = sitemap_url.path().to_string();
        println!("Processing sitemap path: {}", sitemap_path);

        website.with_sitemap(Some(&sitemap_path));
        website.crawl_sitemap().await;

        let website_links: spider::hashbrown::HashSet<CaseInsensitiveString> = website.get_links();
        total_pages += website_links.len();

        for link in &website_links {
            let link_str = link.to_string();
            extracted_links.push(link_str.clone());
            cache_lock.insert("Crawled".to_string(), link_str);
        }
    }

    let duration = start.elapsed();
    let response_body = format!(
        "Sitemap created successfully.\n\nTime elapsed: {:.6} seconds for total pages: {}\n",
        duration.as_secs_f64(),
        total_pages
    );

    println!(
        "Time elapsed in website crawl: {:?} for total pages: {}",
        duration,
        total_pages
    );

    (HttpResponse::Ok().body(response_body), extracted_links)
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

    let (_response, sitemap_links) = create_sitemap(cache.clone()).await;

    let mut conf = TransformConfig::default();
    conf.return_format = ReturnFormat::Markdown;

    let mut cache_lock = cache.lock().unwrap();

    for link in sitemap_links.iter() {

        if cache_lock.contains_key(&link.to_string()) {
            println!("Skipping cached link: {}", link);
            continue;
        }
        cache_lock.insert(link.to_string(), "Crawled".to_string());

        let mut website = Website::new(link.as_ref());
        let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> =
            website.subscribe(0).unwrap();

        let mut stdout: tokio::io::Stdout = tokio::io::stdout();

        // download file
        let download_file = percent_encode(link.as_bytes(), NON_ALPHANUMERIC).to_string();
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

        website.crawl().await;
        website.unsubscribe();

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
    HttpResponse::Ok().body("HTML to Markdown conversion completed successfully.")
}




#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let cache: Cache = Mutex::new(HashMap::new());
    let cache_data = web::Data::new(cache);

    HttpServer::new(move || {
        App::new()
            .app_data(cache_data.clone()) 
            // .route("/create-sitemap", web::get().to(create_sitemap))
            .route("/html_to_markdown", web::get().to(html_to_markdown))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
