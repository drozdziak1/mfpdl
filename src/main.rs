extern crate reqwest;

extern crate futures;
extern crate scraper;

use scraper::{Html, Selector};

use std::io::{self, Write};
use std::process::Command;

const URL: &'static str = "https://www.musicforprogramming.net";

pub fn main() {
    let mut resp = reqwest::get(URL).unwrap();
    if !resp.status().is_success() {
        panic!("Request failed for {}", URL);
    }

    let body = resp.text().unwrap();

    let fragment = Html::parse_document(&body);

    let ep_selec = Selector::parse("#episodes a").expect("Couldn't parse the episode selector");
    let file_selec =
        Selector::parse("div .pad a[href$=mp3]").expect("Couldn't parse the file link selector");

    let mut file_links: Vec<String> = Vec::new();

    for episode in fragment.select(&ep_selec) {
        let subpage = episode.value().attr("href").unwrap();
        let ep_url = &format!("{}{}{}", URL, "/", subpage);
        let mut subpage_resp = reqwest::get(ep_url).unwrap();
        if !subpage_resp.status().is_success() {
            panic!("Request failed for {}", URL);
        }

        let subpage_body = subpage_resp.text().unwrap();

        let subpage_fragment = Html::parse_document(&subpage_body);

        let file_url = subpage_fragment
            .select(&file_selec)
            .nth(0)
            .expect(&format!("Couldn't find file URL in {}", ep_url))
            .value()
            .attr("href")
            .expect(&format!("Failed to get href at {}", ep_url));

        eprintln!("file_url: {}", file_url);
        file_links.push(String::from(file_url));
    }

    let latest = fragment
        .select(&file_selec)
        .nth(0)
        .expect(&format!("Couldn't find file URL in {}", URL))
        .value()
        .attr("href")
        .expect(&format!("Failed to get href at {}", URL));

    eprintln!("latest: {}", latest);
    file_links.push(String::from(latest));


    for link in file_links {
        let fname = link.split("/").last().unwrap();
        print!("Downloading {}... ", fname);
        io::stdout().flush().unwrap();

        Command::new("wget").args(&["-N", &link]).output().expect("Download failed.");
        println!("OK");
    }

}
