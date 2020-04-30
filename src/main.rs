extern crate reqwest;

extern crate futures;
extern crate scraper;

use futures::future;
use scraper::{Html, Selector};
use tokio::process::Command;

use std::iter::Iterator;

const URL: &'static str = "https://www.musicforprogramming.net";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(URL).await?;
    if !resp.status().is_success() {
        panic!("Request failed for {}", URL);
    }

    let body = resp.text().await?;

    let fragment = Html::parse_document(&body);

    let file_selec =
        Selector::parse("div .pad a[href$=mp3]").expect("Couldn't parse the file link selector");

    let latest_url = fragment
        .select(&file_selec)
        .nth(0)
        .expect("Couldn't find latest URL")
        .value()
        .attr("href")
        .expect(&format!("Failed to get href for latest"));

    let fname = latest_url.split("/").last().unwrap();
    println!("Downloading {}... ", fname);

    Command::new("wget").args(&["-N", &latest_url]).output().await?;
    println!("{} OK", fname);

    let ep_selec = Selector::parse("#episodes a").expect("Couldn't parse the episode selector");
    let futs = fragment
        .select(&ep_selec)
        .enumerate()
        .map(|(idx, episode)| async move {
            let file_selec = Selector::parse("div .pad a[href$=mp3]")
                .expect("Couldn't parse the file link selector");

            let subpage = episode.value().attr("href").unwrap();
            let ep_url = &format!("{}/{}", URL, subpage);
            let subpage_resp = reqwest::get(ep_url).await?;

            let body = subpage_resp.text().await?;

            let subpage_fragment = Html::parse_document(&body);

            let file_url = subpage_fragment
                .select(&(file_selec.clone()))
                .nth(0)
                .expect(&format!("Couldn't find file URL for {}", idx + 1))
                .value()
                .attr("href")
                .expect(&format!("Failed to get href for {}", idx + 1));

            let fname = file_url.split("/").last().unwrap();
            println!("Downloading {}... ", fname);

            Command::new("wget").args(&["-N", &file_url]).output().await?;
            println!("{} OK", fname);

            Result::<(), Box<dyn std::error::Error>>::Ok(())
        });

    future::try_join_all(futs).await?;
    Ok(())
}
