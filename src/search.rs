use crate::{evaluate::evaluate, uci::GameTime, EngineReport};
use chess::{Board, ChessMove, Color, MoveGen, Piece, EMPTY};
use chrono::Duration;
use crossbeam_channel::{Receiver, Sender};
use std::{
    sync::{Arc, RwLock},
    thread::JoinHandle,
    time::Instant,
};

const MAX_PLY: u8 = 64;
pub const INFINITY: i16 = 10000;

pub enum EngineToSearch {
    Start(SearchMode),
    Stop,
    Quit,
}

pub enum SearchToEngine {
    BestMove(ChessMove),
    Summary {
        depth: u8,
        seldepth: u8,
        time: Duration,
        cp: i16,
        nodes: u64,
        nps: u64,
        pv: Vec<ChessMove>,
    },
}

pub struct Search {
    handle: Option<JoinHandle<()>>,
    control_tx: Option<Sender<EngineToSearch>>,
}

impl Search {
    pub fn new() -> Search {
        Search {
            handle: None,
            control_tx: None,
        }
    }

    pub fn init(
        &mut self,
        report_tx: Sender<EngineReport>,
        board: Arc<RwLock<Board>>,
        history: Arc<RwLock<Vec<History>>>,
    ) {
        let (control_tx, control_rx) = crossbeam_channel::unbounded();

        let handle = std::thread::spawn(move || {
            let mut quit = false;
            let mut halt = true;

            while !quit {
                let cmd = control_rx.recv().unwrap();

                let mut search_mode = None;

                match cmd {
                    EngineToSearch::Start(sm) => {
                        search_mode = Some(sm);

                        halt = false
                    }
                    EngineToSearch::Stop => halt = true,
                    EngineToSearch::Quit => quit = true,
                }

                if !halt && !quit {
                    let mut refs = SearchRefs {
                        board: Arc::clone(&board),
                        control_rx: &control_rx,
                        report_tx: &report_tx,
                        search_mode: &search_mode.unwrap(),
                        search_state: &mut SearchState::default(),
                        history: Arc::clone(&history),
                    };

                    let (best_move, terminate) = Self::iterative_deepening(&mut refs);

                    let report = SearchToEngine::BestMove(best_move);

                    report_tx.send(EngineReport::Search(report)).unwrap();

                    if let Some(terminate) = terminate {
                        match terminate {
                            SearchTerminate::Stop => {
                                halt = true;
                            }
                            SearchTerminate::Quit => {
                                halt = true;
                                quit = true;
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
        self.control_tx = Some(control_tx);
    }

    pub fn send(&self, cmd: EngineToSearch) {
        if let Some(tx) = &self.control_tx {
            tx.send(cmd).unwrap();
        }
    }

    fn iterative_deepening(refs: &mut SearchRefs) -> (ChessMove, Option<SearchTerminate>) {
        let mut best_move = None;
        let mut root_pv = Vec::new();
        let mut depth = 1;
        let mut stop = false;

        if let SearchMode::GameTime(gametime) = &refs.search_mode {
            let is_white = refs.board.read().unwrap().side_to_move() == Color::White;

            let clock = match is_white {
                true => gametime.white_time,
                false => gametime.black_time,
            };

            let increment = match is_white {
                true => gametime.white_increment,
                false => gametime.black_increment,
            };

            let time = match gametime.moves_to_go {
                Some(moves_to_go) => {
                    if moves_to_go == 0 {
                        clock
                    } else {
                        clock / moves_to_go as i32
                    }
                }
                None => clock / 30,
            };

            let time_slice = time + increment - Duration::milliseconds(100);

            refs.search_state.allocated_time = time_slice.to_std().unwrap_or_default()
        }

        refs.search_state.start_time = Some(Instant::now());

        while depth <= MAX_PLY && !stop {
            refs.search_state.depth = depth;

            let eval = Self::negamax(refs, &mut root_pv, depth, -INFINITY, INFINITY);

            if refs.search_state.terminate.is_none() {
                if !root_pv.is_empty() {
                    best_move = Some(root_pv[0]);
                }

                let elapsed = refs.search_state.start_time.unwrap().elapsed();

                let report = SearchToEngine::Summary {
                    depth,
                    seldepth: refs.search_state.seldepth,
                    time: Duration::from_std(elapsed).unwrap(),
                    cp: eval,
                    nodes: refs.search_state.nodes,
                    nps: (refs.search_state.nodes as f64 / elapsed.as_secs_f64()) as u64,
                    pv: root_pv.clone(),
                };

                refs.report_tx.send(EngineReport::Search(report)).unwrap();

                depth += 1;
            }

            let is_time_up = match refs.search_mode {
                SearchMode::GameTime(_) => {
                    refs.search_state.start_time.unwrap().elapsed()
                        >= refs.search_state.allocated_time
                }
                _ => false,
            };

            if is_time_up || refs.search_state.terminate.is_some() {
                stop = true;
            }
        }

        (best_move.unwrap(), refs.search_state.terminate)
    }

    fn negamax(
        refs: &mut SearchRefs,
        pv: &mut Vec<ChessMove>,
        mut depth: u8,
        mut alpha: i16,
        beta: i16,
    ) -> i16 {
        if refs.search_state.nodes % 0x2000 == 0 {
            check_terminate(refs);
        }

        if refs.search_state.terminate.is_some() {
            return 0;
        }

        if refs.search_state.ply > MAX_PLY {
            return evaluate(&refs.board.read().unwrap());
        }

        refs.search_state.nodes += 1;

        let mut do_pvs = false;

        let is_check = refs.board.read().unwrap().checkers() != &EMPTY;

        if is_check {
            depth += 1;
        }

        if depth == 0 {
            return Self::quiescence(refs, pv, alpha, beta);
        }

        let ordered_moves = move_ordering(refs, pv.get(0).copied());

        let is_game_over = ordered_moves.is_empty();

        for legal in ordered_moves {
            let old_pos = make_move(refs, legal);

            let mut node_pv = Vec::new();

            let mut eval_score = 0;

            if !is_draw(refs) {
                if do_pvs {
                    eval_score = -Self::negamax(refs, &mut node_pv, depth - 1, -alpha - 1, -alpha);

                    if eval_score > alpha && eval_score < beta {
                        eval_score = -Self::negamax(refs, &mut node_pv, depth - 1, -beta, -alpha);
                    }
                } else {
                    eval_score = -Self::negamax(refs, &mut node_pv, depth - 1, -beta, -alpha);
                }
            }

            unmake_move(refs, old_pos);

            if eval_score >= beta {
                return beta;
            }

            if eval_score > alpha {
                alpha = eval_score;

                do_pvs = true;

                pv.clear();
                pv.push(legal);
                pv.append(&mut node_pv);
            }
        }

        if is_game_over {
            if is_check {
                return -INFINITY + refs.search_state.ply as i16;
            } else {
                return 0;
            }
        }

        alpha
    }

    fn quiescence(
        refs: &mut SearchRefs,
        pv: &mut Vec<ChessMove>,
        mut alpha: i16,
        beta: i16,
    ) -> i16 {
        if refs.search_state.nodes & 0x2000 == 0 {
            check_terminate(refs);
        }

        if refs.search_state.terminate.is_some() {
            return 0;
        }

        if refs.search_state.ply > MAX_PLY {
            return evaluate(&refs.board.read().unwrap());
        }

        refs.search_state.nodes += 1;

        let eval = evaluate(&refs.board.read().unwrap());

        if eval >= beta {
            return beta;
        }

        if eval > alpha {
            alpha = eval;
        }

        let mut legal_moves = MoveGen::new_legal(&refs.board.read().unwrap());

        let board = refs.board.read().unwrap();

        let targets = board.color_combined(!board.side_to_move());
        legal_moves.set_iterator_mask(*targets);

        drop(board);

        for legal in legal_moves {
            let old_pos = make_move(refs, legal);

            let mut node_pv = Vec::new();

            let score = -Self::quiescence(refs, &mut node_pv, -beta, -alpha);

            unmake_move(refs, old_pos);

            if score >= beta {
                return beta;
            }

            if score > alpha {
                alpha = score;

                pv.clear();
                pv.push(legal);
                pv.append(&mut node_pv);
            }
        }

        alpha
    }
}

fn move_ordering(refs: &mut SearchRefs, pv: Option<ChessMove>) -> Vec<ChessMove> {
    let board = refs.board.read().unwrap();

    let mut legal_moves = MoveGen::new_legal(&refs.board.read().unwrap());

    let mut moves = Vec::with_capacity(legal_moves.len());

    let targets = board.color_combined(!board.side_to_move());
    legal_moves.set_iterator_mask(*targets);

    for legal in &mut legal_moves {
        if let Some(pv) = pv {
            if legal == pv {
                moves.push((legal, 0));
            }
        } else {
            moves.push((legal, 1));
        }
    }

    legal_moves.set_iterator_mask(!EMPTY);

    for legal in legal_moves {
        if let Some(pv) = pv {
            if legal == pv {
                moves.push((legal, 0));
            }
        } else {
            moves.push((legal, 2));
        }
    }

    moves.sort_unstable_by_key(|(_, score)| *score);

    moves.into_iter().map(|(m, _)| m).collect()
}

fn make_move(refs: &mut SearchRefs, legal: ChessMove) -> Board {
    let old_pos = *refs.board.read().unwrap();

    let new_move = refs.board.read().unwrap().make_move_new(legal);

    *refs.board.write().unwrap() = new_move;

    refs.history.write().unwrap().push(History {
        hash: refs.board.read().unwrap().get_hash(),
        is_reversible_move: old_pos.piece_on(legal.get_dest()).is_some()
            || old_pos.piece_on(legal.get_source()) != Some(Piece::Pawn),
    });

    refs.search_state.ply += 1;

    if refs.search_state.ply > refs.search_state.seldepth {
        refs.search_state.seldepth = refs.search_state.ply;
    }

    old_pos
}

fn unmake_move(refs: &mut SearchRefs, old_pos: Board) {
    refs.search_state.ply -= 1;

    *refs.board.write().unwrap() = old_pos;

    refs.history.write().unwrap().pop();
}

fn check_terminate(refs: &mut SearchRefs) {
    if let Ok(cmd) = refs.control_rx.try_recv() {
        match cmd {
            EngineToSearch::Stop => refs.search_state.terminate = Some(SearchTerminate::Stop),
            EngineToSearch::Quit => refs.search_state.terminate = Some(SearchTerminate::Quit),

            _ => {}
        }
    }

    match refs.search_mode {
        SearchMode::Infinite => {}
        SearchMode::MoveTime(movetime) => {
            if refs.search_state.start_time.unwrap().elapsed().as_millis()
                >= movetime.num_milliseconds() as u128
            {
                refs.search_state.terminate = Some(SearchTerminate::Stop);
            }
        }
        SearchMode::GameTime(_) => {
            if refs.search_state.start_time.unwrap().elapsed() >= refs.search_state.allocated_time {
                refs.search_state.terminate = Some(SearchTerminate::Stop);
            }
        }
    }
}

fn is_draw(refs: &mut SearchRefs) -> bool {
    is_insufficient_material(refs) || is_threefold_repetition(refs) || is_fifty_move_rule(refs)
}

fn is_threefold_repetition(refs: &mut SearchRefs) -> bool {
    let board = refs.board.read().unwrap();

    let mut count = 0;

    for i in 0..refs.history.read().unwrap().len() {
        if refs.history.read().unwrap()[i].hash == board.get_hash() {
            count += 1;
        }
    }

    count >= 3
}

fn is_fifty_move_rule(refs: &mut SearchRefs) -> bool {
    let mut count = 0;

    for i in 0..refs.history.read().unwrap().len() {
        if refs.history.read().unwrap()[i].is_reversible_move {
            count += 1;
        } else {
            count = 0;
        }

        if count >= 100 {
            return true;
        }
    }

    false
}

fn is_insufficient_material(refs: &mut SearchRefs) -> bool {
    let board = refs.board.read().unwrap();

    let white = board.color_combined(Color::White);
    let black = board.color_combined(Color::Black);

    let white_queens = board.pieces(chess::Piece::Queen) & white;
    let black_queens = board.pieces(chess::Piece::Queen) & black;

    if white_queens.popcnt() > 0 || black_queens.popcnt() > 0 {
        return false;
    }

    let white_rooks = board.pieces(chess::Piece::Rook) & white;
    let black_rooks = board.pieces(chess::Piece::Rook) & black;

    if white_rooks.popcnt() > 0 || black_rooks.popcnt() > 0 {
        return false;
    }

    let white_bishops = board.pieces(chess::Piece::Bishop) & white;
    let black_bishops = board.pieces(chess::Piece::Bishop) & black;

    if white_bishops.popcnt() > 0 && black_bishops.popcnt() > 0 {
        return false;
    }

    let white_knights = board.pieces(chess::Piece::Knight) & white;
    let black_knights = board.pieces(chess::Piece::Knight) & black;

    if white_knights.popcnt() > 0 && black_knights.popcnt() > 0 {
        return false;
    }

    let white_pawns = board.pieces(chess::Piece::Pawn) & white;
    let black_pawns = board.pieces(chess::Piece::Pawn) & black;

    if white_pawns.popcnt() > 0 || black_pawns.popcnt() > 0 {
        return false;
    }

    true
}

#[derive(Debug)]
struct SearchRefs<'a> {
    board: Arc<RwLock<Board>>,
    control_rx: &'a Receiver<EngineToSearch>,
    report_tx: &'a Sender<EngineReport>,
    search_mode: &'a SearchMode,
    search_state: &'a mut SearchState,
    history: Arc<RwLock<Vec<History>>>,
}

#[derive(Debug)]
pub struct History {
    pub hash: u64,
    pub is_reversible_move: bool,
}

#[derive(Debug)]
pub enum SearchMode {
    Infinite,
    MoveTime(Duration),
    GameTime(GameTime),
}

#[derive(Debug, Default)]
struct SearchState {
    nodes: u64,
    ply: u8,
    depth: u8,
    seldepth: u8,
    terminate: Option<SearchTerminate>,
    start_time: Option<Instant>,
    allocated_time: std::time::Duration,
}

#[derive(Clone, Copy, Debug)]
enum SearchTerminate {
    Stop,
    Quit,
}
