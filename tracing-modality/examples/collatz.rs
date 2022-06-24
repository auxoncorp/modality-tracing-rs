use rand::{thread_rng, Rng};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{sleep, Builder, JoinHandle};
use std::time::Duration;
use tracing::{event, instrument, span, Level};
use tracing_modality::{timeline_id, TimelineId, TracingModality};

const THREADS: usize = 7;

enum Message {
    Data(Job),
    AllDone,
}

struct Job {
    nonce: u32,
    num: u32,
    timeline_id: TimelineId,
}

fn main() {
    TracingModality::init().expect("init tracing");
    let mut rng = thread_rng();

    let terminal_channel: (Sender<Message>, Receiver<Message>) = channel();
    let mut channels: Vec<(usize, Sender<Message>, Receiver<Message>)> = (0..THREADS)
        .map(|i| {
            let (tx, rx) = channel();
            (i, tx, rx)
        })
        .collect();
    let tx_chans: Vec<Sender<Message>> = channels.iter().map(|(_i, tx, _rx)| tx.clone()).collect();

    let threads: Vec<JoinHandle<()>> = channels.drain(..).map(|(i, _tx, rx)| {
        let term_tx = terminal_channel.0.clone();
        let tx_chans = tx_chans.clone();
        Builder::new()
            .name(format!("Worker%{:02}", i))
            .spawn(move || {
                let timeline_id = timeline_id();
                let _span = span!(Level::INFO, "Receiving").entered();
                while let Ok(msg) = rx.recv() {
                    match msg {
                        Message::Data(job) => {
                            event!(
                                Level::INFO, msg="Received message",
                                interaction.remote_nonce=job.nonce,
                                interaction.remote_timeline_id=?job.timeline_id.get_raw(),
                                job.num
                            );

                            let step = collatz(job.num);
                            event!(Level::INFO, step);
                            if step == 1 {
                                event!(
                                    Level::INFO,
                                    msg="Sending to terminal",
                                    nonce = job.nonce,
                                    step
                                );
                                // println!("Send terminal");
                                term_tx.send(Message::Data(Job {
                                    nonce: job.nonce,
                                    num: 1,
                                    timeline_id,
                                })).unwrap();
                            } else {
                                let target = (step as usize) as usize % THREADS;
                                event!(Level::INFO, msg="Sending to worker", nonce=job.nonce, worker=target, step);
                                tx_chans[target].send(Message::Data(Job {
                                    nonce: job.nonce,
                                    num: step,
                                    timeline_id,
                                })).unwrap();
                            }
                        },
                        Message::AllDone => {
                            event!(Level::WARN, "All done!");
                            break;
                        }
                    }
                    std::thread::yield_now();
                }
            }).unwrap()
    }).collect();

    let timeline_id = timeline_id();

    for i in 0..3 {
        // Don't start with 0 or 1
        let start = rng.gen_range(0..=100) + 2;
        let target = (start as usize) as usize % THREADS;
        let _span = span!(Level::INFO, "Starting Collatz").entered();
        event!(
            Level::INFO,
            msg = "Sending to worker",
            nonce = i,
            worker = target,
            step = start,
        );
        tx_chans[target]
            .send(Message::Data(Job {
                nonce: i,
                num: start,
                timeline_id,
            }))
            .unwrap();
        sleep(Duration::from_secs(1));
    }

    for _ in 0..3 {
        terminal_channel.1.recv().unwrap();
    }

    for t in tx_chans.into_iter() {
        t.send(Message::AllDone).unwrap();
    }

    for t in threads.into_iter() {
        t.join().unwrap();
    }
}

#[instrument]
fn collatz(num: u32) -> u32 {
    sleep(Duration::from_millis(10));
    if (num & 0b1) == 0 {
        num / 2
    } else {
        (num * 3) + 1
    }
}
