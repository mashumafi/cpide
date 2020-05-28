use actix_files as fs;
use actix_web::{middleware::Logger, web, App, Error, HttpResponse, HttpServer};
use difference::{Changeset, Difference};
use log::error;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::{remove_file, File},
    io::prelude::*,
    iter,
    process::{Command, Stdio},
};

#[derive(Debug, Serialize, Deserialize)]
struct InputText {
    text: String,
    delimiter: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Program {
    code: String,
    input: InputText,
    output: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Streams {
    out: String,
    err: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    compiler: Streams,
    output: String,
    diff: String,
}

fn remove_carriage_return(text: String) -> String {
    String::from_utf8(
        text.into_bytes()
            .into_iter()
            .filter(|x| *x != 13 as u8)
            .collect::<Vec<u8>>(),
    )
    .unwrap_or_else(|_| "".to_owned())
}

fn diff_line(symbol: char, line: String) -> String {
    line.split("\n")
        .map(|a| format!("{}{}", symbol, &a))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

async fn compile(program: web::Json<Program>) -> Result<HttpResponse, Error> {
    let mut rng = thread_rng();
    let filename: String = iter::repeat(())
        .map(|()| rng.sample(Alphanumeric))
        .take(7)
        .collect();

    let program_filename = format!("/tmp/{}", filename);
    let code_filename = format!("{}.cpp", program_filename);
    let mut file = File::create(code_filename.clone())?;
    file.write_all(&program.code.clone().into_bytes())?;

    let compiler = Command::new("g++")
        .arg("-std=c++14")
        .arg("-Wall")
        .arg("-O0")
        .arg("-o")
        .arg(program_filename.clone())
        .arg(code_filename.clone())
        .output()
        .expect("Failed to compile program");
    let compiler_out = String::from_utf8(compiler.stdout).unwrap_or("".to_owned());
    let compiler_err = String::from_utf8(compiler.stderr).unwrap_or("".to_owned());

    remove_file(code_filename)?;

    let output_out = program
        .input
        .text
        .split(&program.input.delimiter)
        .collect::<Vec<_>>()
        .par_iter()
        .map(|input| {
            let output = Command::new(program_filename.clone())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn();

            match output {
                Ok(mut output) => {
                    {
                        let stdin = output.stdin.as_mut().expect("Failed to open stdin");
                        stdin
                            .write_all(input.as_bytes())
                            .expect("Failed to write to stdin");
                    }
                    let output = output.wait_with_output();
                    match output {
                        Ok(output) => remove_carriage_return(
                            String::from_utf8(output.stdout).expect("A utf8 String"),
                        ),
                        Err(_) => "".to_owned(),
                    }
                }
                Err(_) => "".to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    match remove_file(program_filename) {
        Ok(()) => {}
        Err(err) => error!("{}", err),
    };

    let mut is_diff = false;
    let mut diff_out = String::new();
    let changeset = Changeset::new(
        &remove_carriage_return(program.output.clone()),
        &output_out,
        "\n",
    );
    for diff in changeset.diffs {
        match diff {
            Difference::Add(add) => {
                is_diff |= !add.is_empty();
                diff_out += &diff_line('+', add)
            }
            Difference::Rem(rem) => {
                is_diff |= !rem.is_empty();
                diff_out += &diff_line('-', rem)
            }
            Difference::Same(same) => diff_out += &diff_line(' ', same),
        }
    }
    if !is_diff {
        diff_out = "".to_owned();
    }

    let output = Output {
        compiler: Streams {
            out: compiler_out,
            err: compiler_err,
        },
        output: output_out,
        diff: diff_out,
    };
    Ok(HttpResponse::Ok().json(output))
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    std::env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();

    HttpServer::new(|| {
        App::new()
            .wrap(Logger::default())
            .route("/compile", web::post().to(compile))
            .service(fs::Files::new("/", "./static_html").show_files_listing())
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::dev::Service;
    use actix_web::{http, test, web, App};

    #[test]
    fn test_remove_carriage_return() {
        assert_eq!(
            remove_carriage_return("Hello\r\nWorld\r\n".to_owned()),
            "Hello\nWorld\n"
        );
    }

    #[actix_rt::test]
    async fn test_compile() -> Result<(), Error> {
        let mut app = test::init_service(
            App::new().service(web::resource("/compile").route(web::post().to(compile))),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/compile")
            .set_json(&Program {
                code: r##"
                    #include <iostream>
                    using namespace std;
                    int main()
                    {
                        cout << "Hello\n";
                        return 0;
                    }
                "##
                .to_owned(),
                input: InputText {
                    text: "".to_owned(),
                    delimiter: "\n\n".to_owned(),
                },
                output: "Hello\r\n".to_owned(),
            })
            .to_request();
        let resp = app.call(req).await.unwrap();

        assert_eq!(resp.status(), http::StatusCode::OK);

        let response_body = match resp.response().body().as_ref() {
            Some(actix_web::body::Body::Bytes(bytes)) => bytes,
            _ => panic!("Response error"),
        };

        assert_eq!(
            response_body,
            r##"{"compiler":{"out":"","err":""},"output":"Hello\n","diff":""}"##
        );

        Ok(())
    }

    #[actix_rt::test]
    async fn test_compile_input() -> Result<(), Error> {
        let mut app = test::init_service(
            App::new().service(web::resource("/compile").route(web::post().to(compile))),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/compile")
            .set_json(&Program {
                code: r##"
                    #include <iostream>

                    using namespace std;

                    int main()
                    {
                        int c; cin >> c;
                        for (int i = 0; i < c; ++i)
                        {
                            int v; cin >> v;
                            cout << v * v << "\n";
                        }
                        return 0;
                    }
                "##
                .to_owned(),
                input: InputText {
                    text: "5 1 2 3 4 5".to_owned(),
                    delimiter: "\n\n".to_owned(),
                },
                output: "1\r\n4\r\n9\r\n16\r\n25\r\n".to_owned(),
            })
            .to_request();
        let resp = app.call(req).await.unwrap();

        assert_eq!(resp.status(), http::StatusCode::OK);

        let response_body = match resp.response().body().as_ref() {
            Some(actix_web::body::Body::Bytes(bytes)) => bytes,
            _ => panic!("Response error"),
        };

        assert_eq!(
            response_body,
            r##"{"compiler":{"out":"","err":""},"output":"1\n4\n9\n16\n25\n","diff":""}"##
        );

        Ok(())
    }
}
