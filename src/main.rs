extern crate spider;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use ::futures::future::join_all;
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::{HashMap, HashSet};
use spider::percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use spider::url::Url;
use spider::website::Website;
use spider::{ reqwest, tokio, CaseInsensitiveString};
use spider_utils::spider_transformations::transform_content;
use spider_utils::spider_transformations::transformation::content::{
    ReturnFormat, TransformConfig,
};
use tokio::task::futures;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::AsyncWriteExt;
type Cache = Mutex<HashMap<String, String>>;

pub(crate) fn spawn_task<F>(_task_name: &str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(future)
}
pub async fn create_sitemap() -> (HttpResponse, Vec<String>) {
    let web_url = "https://www.heygoody.com";
    let is_spa = detect_spa(web_url).await;

    let website = if is_spa {
        let mut web_spa = Website::new(web_url);
        let parsed_url = Url::parse(web_url).ok().map(Box::new);
        web_spa.configuration
            .with_respect_robots_txt(true)
            .with_chrome_intercept(RequestInterceptConfiguration::new(true), &parsed_url)
            .with_caching(true);
        web_spa
    } else {
        let mut web_ssr = Website::new(web_url);
        web_ssr.configuration
            .with_respect_robots_txt(true)
            .with_user_agent(Some("SpiderBot"))
            .with_caching(true);
        web_ssr
    };

    let start = Instant::now();

    let sitemap_links = vec![
        "/sitemap.xml",
        "/th/sitemap_index.xml",
        "/th/post-sitemap.xml",
        "/th/page-sitemap.xml",
        "/th/post-sitemap1.xml",
        "/th/post-sitemap2.xml",
        "/th/local-sitemap.xml",
    ];

    let mut tasks = vec![];
    let mut extracted_links = vec![];

    for link in sitemap_links {
        let link = link.to_string();
        let mut website_clone = website.clone();
        let task_start = Instant::now();

        tasks.push(spawn_task("sitemap_crawl_task", async move {
            println!("Task started for link: {} at {:?}", link, task_start);

            website_clone.with_sitemap(Some(&link));
            website_clone.crawl_sitemap().await;

            let website_links: HashSet<CaseInsensitiveString> = website_clone.get_links();
            let mut link_vec = vec![];

            for extracted_link in &website_links {
                println!("Extracted link from {}: {}", link, extracted_link);
                link_vec.push(extracted_link.to_string());
            }

            println!(
                "Task completed for link: {} with {} links in {:.6} seconds",
                link,
                link_vec.len(),
                task_start.elapsed().as_secs_f64()
            );

            link_vec // ส่งกลับ Vec<String> ของลิงก์ที่ดึงได้
        }));
    }

    let mut total_pages = 0;

    // รอผลลัพธ์ของแต่ละ Task
    for task in tasks {
        if let Ok(links) = task.await {
            println!("Task completed with {} links", links.len());
            total_pages += links.len();
            extracted_links.extend(links);
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
    let response = spider::reqwest::get(url).await.unwrap();
    let body = response.text().await.unwrap();
    //detect html content
    body.contains("<script>") || body.contains("fetch(") || body.contains("window.onload")
}

async fn html_to_markdown() -> impl Responder {
    println!("Creating target/downloads directory...");
    std::fs::create_dir_all("./target/downloads").unwrap_or_default();

    let (_response, sitemap_links) = create_sitemap().await;

    let mut conf = TransformConfig::default();
    conf.return_format = ReturnFormat::Markdown;

    let mut tasks = vec![];

    for link in sitemap_links.iter() {
        let link = link.to_string();
        let conf = conf.clone();

        let task = spawn_task("markdown_conversion_task", async move {
            let task_start = Instant::now();
            println!("Task html_to_markdown started for link: {} at {:?}", link, task_start);
            let mut website = Website::new(&link);
            let mut rx2 = website.subscribe(0).unwrap();

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
                }
            });

            // wait for the task to complete
            join_handle.await.unwrap();
            website.crawl().await;
            website.unsubscribe();

        });

        tasks.push(task);
    }

    // wait for all tasks to complete
    join_all(tasks).await;

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
