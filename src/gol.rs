use std::thread;
use std::sync::{Arc, RwLock, Barrier, mpsc};
use kiss3d::ncollide3d::math::Translation;
use kiss3d::scene::SceneNode;
use rand::{self, prelude::*};

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Cell {
    Alive,
    Dead
}

impl Cell {
    pub fn randomized_vec(alive_chance: f32, ct: usize) -> Vec<Cell> {
        let mut res = vec![];
        let mut rng = thread_rng();
        for _ in 0..ct {
            res.push(if rng.gen::<f32>() < alive_chance { Cell::Alive } else { Cell::Dead });
        }
        res
    }
}

// represents a start & end index into the gol mpa & a vector of those computed cells
pub struct GolComputation {
    pub start: usize,
    pub step: usize,
    pub data: Vec<Cell>,
}

impl GolComputation {
    pub fn new(start: usize, step: usize) -> Self {
        GolComputation {
            start,
            step,
            data: vec![]
        }
    }
}

pub struct GameOfLife {
    rules: Arc<GolRules>,
    map: Arc<RwLock<Vec<Cell>>>,
    map_size: usize,
    threads: Vec<thread::JoinHandle<()>>,
    result_channel: (mpsc::Sender<GolComputation>, mpsc::Receiver<GolComputation>),
    compute_lock: Arc<Barrier>,
    node: SceneNode,
    cubes: Vec<SceneNode>,
}

// convert an x, y, z position into a 1d array index
pub fn pos3_to_index(x: i32, y: i32, z: i32, side_len: i32) -> i32 {
    z + side_len * (y + side_len * x)
}

pub fn index_to_pos3(i: i32, side_len: i32) -> (i32, i32, i32) {
    let (x, rem) = (i / side_len.pow(2), i % side_len.pow(2)); 
    let (y, rem) = (rem / side_len, rem % side_len);
    let z = rem;
    (x, y, z)
}

pub struct GolRules {
    pub percent_alive: f32,
    pub kill: Box<dyn Fn(i32) -> bool + Send + Sync + 'static>,
    pub resurrect: Box<dyn Fn(i32) -> bool + Send + Sync + 'static>,
    pub wrap_around: bool,
    pub diagonal_neighbors: bool,
}

trait WrapsAround {
    fn wrap(self, lower: Self, upper: Self) -> Self;
}

impl WrapsAround for i32 {
    fn wrap(mut self, lower: i32, upper: i32) -> i32 {
        while self < lower {
            self += upper - lower;
        }
        while self > upper {
            self -= upper - lower;
        }
        self
    }
}

impl GameOfLife {
    // instantiate a blank game of life map
    pub fn new(thread_count: usize, map_size: usize, node: SceneNode, rules: GolRules) -> GameOfLife {
        let data = Arc::new(RwLock::new(Cell::randomized_vec(rules.percent_alive, map_size.pow(3))));
        let result_channel = mpsc::channel();
        let compute_lock = Arc::new(Barrier::new(thread_count + 1)); // +1 so that step() can unblock all threads at once

        let rules = Arc::new(rules);

        let mut threads = vec![];
        for i in 0..thread_count {
            let cl = compute_lock.clone();
            let sender = result_channel.0.clone();
            let data = Arc::clone(&data);
            let rules = Arc::clone(&rules);
            threads.push(thread::spawn(move || {
                let thread_nb = i; // going to compute cell thread_nb, thread_nb+thread_count, thread_nb+2*thread_count, etc
                //println!("in thread: {}", thread_nb);

                loop {
                    cl.wait(); // wait for computation to be ready
                    let cells: &Vec<Cell> = &*data.read().unwrap(); // get cells

                    let mut computation = GolComputation::new(thread_nb, thread_count);

                    for (i, cell) in cells.iter().enumerate().skip(thread_nb).step_by(thread_count) {
                        //println!("thread nb {} computing cell {}", thread_nb, i);
                        let map_size = map_size as i32;

                        // cell position
                        let (x, y, z) = index_to_pos3(i as i32, map_size);

                        // get neighboring states
                        let mut alive_neighbors = 0;
                        let mut _dead_neighbors = 0;
                        for dx in -1..=1 {
                            for dy in -1..=1 {
                                for dz in -1..=1 {
                                    if dx == 0 && dy == 0 && dz == 0 {
                                        continue;
                                    }
                                    if !rules.diagonal_neighbors { 
                                        // only one of dx, dy, or dz can be non-zero at any time
                                        if dx != 0 && (dy != 0 || dz != 0) { continue; }
                                        if dy != 0 && (dx != 0 || dz != 0) { continue; }
                                        if dz != 0 && (dx != 0 || dy != 0) { continue; }
                                    }

                                    let idx = if rules.wrap_around {
                                        pos3_to_index(
                                            (x + dx).wrap(0, map_size - 1),
                                            (y + dy).wrap(0, map_size - 1),
                                            (z + dz).wrap(0, map_size - 1),
                                            map_size
                                        )
                                    } else {
                                        pos3_to_index(
                                            x + dx,
                                            y + dy,
                                            z + dz,
                                            map_size
                                        )
                                    };

                                    match cells.get(idx as usize) {
                                        Some(Cell::Alive) => alive_neighbors += 1,
                                        Some(Cell::Dead) => _dead_neighbors += 1,
                                        None => _dead_neighbors += 1, // treat oob as dead
                                    }
                                }
                            }
                        }
                        // apply rules
                        let new_cell = match cell {
                            Cell::Alive => if (rules.kill)(alive_neighbors) { Cell::Dead } else { Cell::Alive },
                            Cell::Dead => if (rules.resurrect)(alive_neighbors) { Cell::Alive } else { Cell::Dead }
                        };
                        computation.data.push(new_cell);
                    }
                    // send resulting computation
                    sender.send(computation).unwrap();
                }

            }));
        }
        let mut res = GameOfLife {
            map: data.clone(),
            map_size,
            threads,
            result_channel,
            compute_lock,
            node,
            rules,
            cubes: vec![SceneNode::new_empty(); map_size.pow(3)],
        };
        // generate map_size^3 cubes and compute their visibility
        res.compute_cubes(true);
        res
    }

    // steps the gol simulation
    pub fn step(&mut self) {
        self.compute_lock.wait(); // wait for all threads to begin computation
        
        let mut new_map = vec![Cell::Dead; self.map_size.pow(3)];
        // we expect to receive thread_count computations
        let receiver = &self.result_channel.1;
        for _ in 0..self.threads.len() {
            // fetch computation
            let GolComputation{ start, step, data } = receiver.recv().expect("failed to receive thread data");
            // insert into resulting data
            for (i, cell) in data.iter().enumerate() {
                if let Some(cell_elem) = new_map.get_mut(start + i * step) {
                    *cell_elem = *cell;
                } else {
                    panic!("gol oob");
                }
            }
        }
        // once all computations are in, assign the result
        if let Ok(mut wmap) = self.map.write() {
            *wmap = new_map;
        } else {
            panic!("could not write computation result");
        }
        self.compute_cubes(false);
    }

    pub fn alive(&self) -> usize {
        self.map.read()
            .unwrap()
            .iter()
            .filter(|&c| c == &Cell::Alive)
            .count()
    }

    // computes vertices and faces for the map
    pub fn compute_cubes(&mut self, init: bool) {
        let side_len = self.map_size as i32;
        let cells: &Vec<Cell> = &*self.map.read().unwrap();
        // every cell is a cube
        for x in 0..side_len {
            for y in 0..side_len {
                for z in 0..side_len {
                    let (xf, yf, zf) = (x as f32, y as f32, z as f32);
                    if init {
                        let mut cube = self.node.add_cube(1.0, 1.0, 1.0);
                        cube.append_translation(&Translation::new(x as f32, y as f32, z as f32));
                        cube.set_color(xf / side_len as f32, yf / side_len as f32, zf / side_len as f32);
                        self.cubes[pos3_to_index(x, y, z, side_len) as usize] = cube;
                    }
                    // only process alive cells
                    if let Some(&cell) = cells.get(pos3_to_index(x, y, z, side_len) as usize) {
                        let cube = &mut self.cubes[pos3_to_index(x, y, z, side_len) as usize];
                        match cell {
                            Cell::Alive => {
                                cube.set_visible(true);
                            },
                            Cell::Dead => {
                                cube.set_visible(false);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_pos3_conversions() {
    let side_len = 10;
    let mut expected_idx = 0;
    for x in 0..10 {
        for y in 0..10 {
            for z in 0..10 {
                let i = pos3_to_index(x, y, z, side_len);
                assert_eq!(expected_idx, i);
                assert_eq!((x, y, z), index_to_pos3(i, side_len));
                expected_idx += 1;
            }
        }
    }
}
