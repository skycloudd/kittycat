use crate::{search::INFINITY, EngineReport};
use chess::ChessMove;
use chrono::Duration;
use crossbeam_channel::Sender;
use std::thread::JoinHandle;
use vampirc_uci::{UciInfoAttribute, UciMessage, UciTimeControl};

pub enum EngineToUci {
    Identify,
    Ready,
    Quit,
    BestMove(ChessMove),
    Summary {
        depth: u8,
        seldepth: u8,
        time: Duration,
        cp: i16,
        nodes: u64,
        nps: u64,
        pv: Vec<chess::ChessMove>,
    },
}

pub enum UciToEngine {
    Uci,
    Debug(bool),
    IsReady,
    Register,
    Position(String, Vec<ChessMove>),
    SetOption,
    UciNewGame,
    Stop,
    PonderHit,
    Quit,
    GoInfinite,
    GoMoveTime(Duration),
    GoGameTime(GameTime),
    Unknown,
}

pub struct Uci {
    report_handle: Option<JoinHandle<()>>,
    control_handle: Option<JoinHandle<()>>,
    control_tx: Option<Sender<EngineToUci>>,
}

impl Uci {
    pub fn new() -> Uci {
        Uci {
            control_handle: None,
            report_handle: None,
            control_tx: None,
        }
    }

    pub fn init(&mut self, report_tx: Sender<EngineReport>) {
        self.report_thread(report_tx);
        self.control_thread();
    }

    pub fn send(&mut self, msg: EngineToUci) {
        if let Some(tx) = &self.control_tx {
            tx.send(msg).unwrap();
        }
    }

    fn report_thread(&mut self, report_tx: Sender<EngineReport>) {
        let mut incoming_data = String::new();

        let report_handle = std::thread::spawn(move || {
            let mut quit = false;

            while !quit {
                std::io::stdin().read_line(&mut incoming_data).unwrap();

                let msgs = vampirc_uci::parse(&incoming_data);

                for msg in msgs {
                    let report = match msg {
                        UciMessage::Uci => UciToEngine::Uci,

                        UciMessage::Debug(debug) => UciToEngine::Debug(debug),

                        UciMessage::IsReady => UciToEngine::IsReady,

                        UciMessage::Register {
                            later: _,
                            name: _,
                            code: _,
                        } => UciToEngine::Register,

                        UciMessage::Position {
                            startpos,
                            fen,
                            moves,
                        } => {
                            let fen = if startpos {
                                String::from(
                                    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                                )
                            } else {
                                fen.unwrap().to_string()
                            };

                            UciToEngine::Position(fen, moves)
                        }

                        UciMessage::SetOption { name: _, value: _ } => UciToEngine::SetOption,

                        UciMessage::UciNewGame => UciToEngine::UciNewGame,

                        UciMessage::Stop => UciToEngine::Stop,

                        UciMessage::PonderHit => UciToEngine::PonderHit,

                        UciMessage::Quit => {
                            quit = true;

                            UciToEngine::Quit
                        }

                        UciMessage::Go {
                            time_control,
                            search_control,
                        } => {
                            if let Some(time_control) = time_control {
                                match time_control {
                                    UciTimeControl::Ponder => panic!("ponder not implemented"),
                                    UciTimeControl::Infinite => UciToEngine::GoInfinite,
                                    UciTimeControl::TimeLeft {
                                        white_time,
                                        black_time,
                                        white_increment,
                                        black_increment,
                                        moves_to_go,
                                    } => UciToEngine::GoGameTime(GameTime {
                                        white_time: white_time.unwrap(),
                                        black_time: black_time.unwrap(),
                                        white_increment: white_increment.unwrap(),
                                        black_increment: black_increment.unwrap(),
                                        moves_to_go,
                                    }),
                                    UciTimeControl::MoveTime(movetime) => {
                                        UciToEngine::GoMoveTime(movetime)
                                    }
                                }
                            } else if let Some(_) = search_control {
                                todo!()
                            } else {
                                unreachable!()
                            }
                        }

                        _ => UciToEngine::Unknown,
                    };

                    report_tx.send(EngineReport::Uci(report)).unwrap();
                }

                incoming_data.clear();
            }
        });

        self.report_handle = Some(report_handle);
    }

    fn control_thread(&mut self) {
        let (control_tx, control_rx) = crossbeam_channel::unbounded();

        let control_handle = std::thread::spawn(move || {
            let mut quit = false;

            while !quit {
                let msg = control_rx.recv().unwrap();

                match msg {
                    EngineToUci::Identify => {
                        println!("{}", UciMessage::id_name("kittycat"));
                        println!("{}", UciMessage::id_author("skycloudd"));
                        println!("{}", UciMessage::UciOk);
                    }
                    EngineToUci::Ready => println!("{}", UciMessage::ReadyOk),
                    EngineToUci::Quit => quit = true,
                    EngineToUci::BestMove(bestmove) => {
                        println!("{}", UciMessage::best_move(bestmove));
                    }
                    EngineToUci::Summary {
                        depth,
                        seldepth,
                        time,
                        cp,
                        nodes,
                        nps,
                        pv,
                    } => {
                        let (cp, mate) = if cp.abs() > INFINITY / 2 {
                            let mate_in_plies = INFINITY - cp.abs();
                            let sign = cp.signum();

                            let mate_in_moves = mate_in_plies / 2 + mate_in_plies % 2;

                            (None, Some((sign * mate_in_moves) as i8))
                        } else {
                            (Some(cp), None)
                        };

                        println!(
                            "{}",
                            UciMessage::Info(vec![
                                UciInfoAttribute::Depth(depth),
                                UciInfoAttribute::SelDepth(seldepth),
                                UciInfoAttribute::Time(time),
                                UciInfoAttribute::Score {
                                    cp: cp.map(|cp| cp as i32),
                                    mate,
                                    lower_bound: None,
                                    upper_bound: None
                                },
                                UciInfoAttribute::Nodes(nodes),
                                UciInfoAttribute::Nps(nps),
                                UciInfoAttribute::Pv(pv)
                            ])
                        )
                    }
                }
            }
        });

        self.control_handle = Some(control_handle);
        self.control_tx = Some(control_tx);
    }
}

#[derive(Debug)]
pub struct GameTime {
    pub white_time: Duration,
    pub black_time: Duration,
    pub white_increment: Duration,
    pub black_increment: Duration,
    pub moves_to_go: Option<u8>,
}
