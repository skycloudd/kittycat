use chess::{Board, Piece};
use search::{EngineToSearch, History, Search, SearchMode, SearchToEngine};
use std::{
    str::FromStr,
    sync::{Arc, RwLock},
};
use uci::{EngineToUci, Uci, UciToEngine};

mod evaluate;
mod search;
mod uci;

fn main() {
    Engine::new().main_loop();
}

struct Engine {
    board: Arc<RwLock<Board>>,
    uci: Uci,
    search: Search,
    quit: bool,
    debug: bool,
}

impl Engine {
    fn new() -> Engine {
        Engine {
            board: Arc::new(RwLock::new(Board::default())),
            uci: Uci::new(),
            search: Search::new(),
            quit: false,
            debug: false,
        }
    }

    fn main_loop(&mut self) {
        let (report_tx, report_rx) = crossbeam_channel::unbounded();

        let history = Arc::new(RwLock::new(Vec::new()));

        self.uci.init(report_tx.clone());
        self.search
            .init(report_tx, Arc::clone(&self.board), Arc::clone(&history));

        while !self.quit {
            match report_rx.recv().unwrap() {
                EngineReport::Uci(uci_report) => match uci_report {
                    UciToEngine::Uci => self.uci.send(EngineToUci::Identify),
                    UciToEngine::Debug(debug) => self.debug = debug,
                    UciToEngine::IsReady => self.uci.send(EngineToUci::Ready),
                    UciToEngine::Register => panic!("register not implemented"),
                    UciToEngine::Position(fen, moves) => {
                        let mut board = self.board.write().unwrap();
                        let mut history = history.write().unwrap();

                        *board = Board::from_str(&fen).unwrap();
                        *history = Vec::new();

                        for m in moves {
                            let old_pos = *board;
                            *board = board.make_move_new(m);

                            history.push(History {
                                hash: board.get_hash(),
                                is_reversible_move: old_pos.piece_on(m.get_dest()).is_some()
                                    || old_pos.piece_on(m.get_source()) != Some(Piece::Pawn),
                            });
                        }
                    }
                    UciToEngine::SetOption => panic!("setoption not implemented"),
                    UciToEngine::UciNewGame => {
                        *self.board.write().unwrap() = Board::default();
                        *history.write().unwrap() = Vec::new();
                    }
                    UciToEngine::Stop => self.search.send(EngineToSearch::Stop),
                    UciToEngine::PonderHit => panic!("ponderhit not implemented"),
                    UciToEngine::Quit => self.quit(),
                    UciToEngine::GoInfinite => self
                        .search
                        .send(EngineToSearch::Start(SearchMode::Infinite)),
                    UciToEngine::GoMoveTime(movetime) => self
                        .search
                        .send(EngineToSearch::Start(SearchMode::MoveTime(movetime))),
                    UciToEngine::GoGameTime(gametime) => self
                        .search
                        .send(EngineToSearch::Start(SearchMode::GameTime(gametime))),
                    UciToEngine::Unknown => {}
                },
                EngineReport::Search(search_report) => match search_report {
                    SearchToEngine::BestMove(bestmove) => {
                        self.uci.send(EngineToUci::BestMove(bestmove))
                    }
                    search::SearchToEngine::Summary {
                        depth,
                        seldepth,
                        time,
                        cp,
                        nodes,
                        nps,
                        pv,
                    } => self.uci.send(EngineToUci::Summary {
                        depth,
                        seldepth,
                        time,
                        cp,
                        nodes,
                        nps,
                        pv,
                    }),
                },
            }
        }
    }

    fn quit(&mut self) {
        self.uci.send(EngineToUci::Quit);
        self.search.send(EngineToSearch::Quit);

        self.quit = true;
    }
}

pub enum EngineReport {
    Uci(UciToEngine),
    Search(SearchToEngine),
}
