use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bit_vec::BitVec;
use crossbeam_channel::{Receiver, Sender, unbounded};
use noise::{NoiseFn, Point2, Point3, Seedable, SuperSimplex};
use num_traits::abs;
use parking_lot::RwLock;
use specs::{Join, Read, ReadStorage, System};

use crate::chunk::{BlockID, BlockIterator, Chunk, ChunkColumn};
use crate::chunk_manager::ChunkManager;
use crate::constants::{CHUNK_UPLOADS_PER_FRAME, RENDER_DISTANCE, WORLD_GENERATION_THREAD_POOL_SIZE, WORLD_SEED};
use crate::physics::Interpolator;
use crate::player::PlayerPhysicsState;
use crate::types::TexturePack;

#[derive(Eq)]
struct PrioritizedItem<T> {
    pub item: T,
    pub priority: i32,
}

impl<T: Eq> Ord for PrioritizedItem<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl<T: Eq> PartialOrd for PrioritizedItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> PartialEq for PrioritizedItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl<T> Deref for PrioritizedItem<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.item
    }
}

pub struct ChunkLoading {
    noise_fn: SuperSimplex,
    chunk_column_pool: Arc<RwLock<Vec<Arc<ChunkColumn>>>>,

    request_chunk_columns_tx: Sender<()>,
    request_chunk_columns_rx: Receiver<()>,

    requested_chunk_column_tx: Sender<Arc<ChunkColumn>>,
    requested_chunk_column_rx: Receiver<Arc<ChunkColumn>>,

    upload_chunks_tx: Sender<PrioritizedItem<(i32, i32, i32)>>,
    upload_chunks_rx: Receiver<PrioritizedItem<(i32, i32, i32)>>,

    chunk_upload_priority_queue: BinaryHeap<PrioritizedItem<(i32, i32, i32)>>,

    expand_chunks: Arc<RwLock<bool>>,
    world_generation_thread_pool: rayon::ThreadPool,
    player_interaction_thread_pool: rayon::ThreadPool,
}

fn compute_tree_placement_in_chunk(noise: &SuperSimplex, x: f64, z: f64) -> Vec<(u32, u32)> {
    let mut maximums = Vec::new();

    #[inline]
    fn index(i: i32, j: i32) -> usize {
        (18 * i + j) as usize
    }

    let mut samples: [f64; 18 * 18] = [0.0; 18 * 18];
    for i in -1..=16 {
        for j in -1..=16 {
            let x = x + j as f64 * 0.075;
            let z = z + i as f64 * 0.075;
            samples[index(i + 1, j + 1)] = noise.get(Point2::from([x, z]))
        }
    }

    for i in 1..17 {
        for j in 1..17 {
            let center = samples[index(i, j)];
            let is_max = (|| {
                for ni in i - 1..=i + 1 {
                    for nj in j - 1..=j + 1 {
                        if ni == i && nj == j {
                            continue;
                        }
                        if samples[index(ni, nj)] >= center {
                            return false;
                        }
                    }
                }
                return true;
            })();
            if is_max {
                maximums.push(((j - 1) as u32, (i - 1) as u32));
            }
        }
    }
    maximums
}

impl ChunkLoading {
    pub fn new() -> Self {
        let (request_chunk_column_tx, request_chunk_column_rx) = unbounded();
        let (requested_chunk_column_tx, requested_chunk_column_rx) = unbounded();
        let (upload_chunks_tx, upload_chunks_rx) = unbounded();

        Self {
            noise_fn: {
                let mut ss = SuperSimplex::new();
                ss = ss.set_seed(*WORLD_SEED);
                ss
            },
            chunk_column_pool: Arc::new(RwLock::new({
                let mut vec = Vec::new();
                let matrix_width = (2 * (RENDER_DISTANCE + 2) + 1) as usize;

                let reserved_columns = matrix_width * matrix_width;
                vec.reserve(reserved_columns);
                for _ in 0..reserved_columns {
                    vec.push(Arc::new(ChunkColumn::new()));
                }
                vec
            })),
            request_chunk_columns_tx: request_chunk_column_tx,
            request_chunk_columns_rx: request_chunk_column_rx,
            requested_chunk_column_tx,
            requested_chunk_column_rx,
            upload_chunks_tx,
            upload_chunks_rx,
            chunk_upload_priority_queue: BinaryHeap::new(),
            expand_chunks: Arc::new(RwLock::new(true)),
            world_generation_thread_pool: rayon::ThreadPoolBuilder::new()
                .stack_size(4 * 1024 * 1024)
                .num_threads(*WORLD_GENERATION_THREAD_POOL_SIZE)
                .build().unwrap(),
            player_interaction_thread_pool: rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build().unwrap(),
        }
    }

    fn flood_fill_unloaded_columns(chunk_manager: &ChunkManager, x: i32, z: i32, distance: i32) -> Vec<(i32, i32)> {
        assert!(distance >= 2);

        let matrix_width = 2 * distance + 1;
        let mut is_visited = BitVec::from_elem(
            (matrix_width * matrix_width) as usize, false);

        let center = (x, z);
        let matrix_index = move |x: i32, z: i32| {
            (matrix_width * (x - center.0 + distance)
                + (z - center.1 + distance)) as usize
        };

        let is_position_valid = |c_x: i32, c_z: i32| {
            abs(x - c_x) <= distance && abs(z - c_z) <= distance
        };

        let mut queue = VecDeque::new();
        let mut ring = Vec::new();
        let mut ring_number = 0;

        queue.push_back((x, z));
        ring.push((x, z));
        is_visited.set(matrix_index(x, z), true);

        while !queue.is_empty() {
            // Expand the ring
            for (c_x, c_z) in queue.drain(..) {
                for &(c_x, c_z) in &[
                    (c_x + 1, c_z),
                    (c_x - 1, c_z),
                    (c_x, c_z + 1),
                    (c_x, c_z - 1),
                ] {
                    if is_position_valid(c_x, c_z) && !is_visited[matrix_index(c_x, c_z)] {
                        ring.push((c_x, c_z));
                        is_visited.set(matrix_index(c_x, c_z), true);
                    }
                }
            }

            // We must expand at least 2 rings before returning something
            ring_number += 1;
            if ring_number < 2 {
                queue.extend(ring.iter());
                continue;
            }

            let mut unloaded_columns = Vec::new();
            for column in &ring {
                if !chunk_manager.loaded_chunk_columns.read().contains_key(column) {
                    unloaded_columns.push(*column);
                }
            }
            if !unloaded_columns.is_empty() {
                return unloaded_columns;
            } else {
                queue.extend(ring.iter());
                ring.clear();
            }
        }
        Vec::new()
    }

    fn flood_fill_unfoliated_columns(chunk_manager: &ChunkManager, x: i32, z: i32, distance: i32) -> Vec<(i32, i32)> {
        assert!(distance >= 0);

        let matrix_width = 2 * distance + 1;
        let mut is_visited = BitVec::from_elem(
            (matrix_width * matrix_width) as usize, false);

        let center = (x, z);
        let matrix_index = move |x: i32, z: i32| {
            (matrix_width * (x - center.0 + distance)
                + (z - center.1 + distance)) as usize
        };

        let is_position_valid = |c_x: i32, c_z: i32| {
            abs(x - c_x) <= distance && abs(z - c_z) <= distance
        };

        let mut queue = VecDeque::new();
        let mut ring = Vec::new();

        queue.push_back((x, z));
        ring.push((x, z));
        is_visited.set(matrix_index(x, z), true);

        // First column
        if let Some(column) = chunk_manager.get_column(x, z) {
            if !*column.has_foliage.read() {
                return ring;
            }
        }

        while !queue.is_empty() {
            // Expand the ring
            for (c_x, c_z) in queue.drain(..) {
                for &(c_x, c_z) in &[
                    (c_x + 1, c_z),
                    (c_x - 1, c_z),
                    (c_x, c_z + 1),
                    (c_x, c_z - 1),
                ] {
                    if is_position_valid(c_x, c_z) && !is_visited[matrix_index(c_x, c_z)] {
                        ring.push((c_x, c_z));
                        is_visited.set(matrix_index(c_x, c_z), true);
                    }
                }
            }

            let mut unfoliated_columns = Vec::new();

            for &(x, z) in &ring {
                let has_foliage = match chunk_manager.get_column(x, z) {
                    Some(column) => *column.has_foliage.read(),
                    None => {
                        true // Hack to stop spreading
                    }
                };
                if !has_foliage {
                    unfoliated_columns.push((x, z));
                }
            }
            if !unfoliated_columns.is_empty() {
                return unfoliated_columns;
            } else {
                queue.extend(ring.iter());
                ring.clear();
            }
        }
        Vec::new()
    }

    fn flood_fill_chunks(chunk_manager: &ChunkManager, x: i32, y: i32, z: i32, distance: i32) -> Vec<(i32, i32, i32)> {
        assert!(distance >= 0);

        let matrix_width = 2 * distance + 1;
        let mut is_visited = BitVec::from_elem(
            (matrix_width * matrix_width * matrix_width) as usize, false);

        let center = (x, y, z);
        let coords_to_index = move |x: i32, y: i32, z: i32| {
            (matrix_width * matrix_width * (x - center.0 + distance)
                + matrix_width * (y - center.1 + distance)
                + (z - center.2 + distance)) as usize
        };

        let is_position_valid = |c_x: i32, c_y: i32, c_z: i32| {
            abs(x - c_x) <= distance &&
                abs(y - c_y) <= distance &&
                abs(z - c_z) <= distance
        };

        let mut queue = VecDeque::new();
        let mut ring = Vec::new();

        queue.push_back((x, y, z));
        ring.push((x, y, z));
        is_visited.set(coords_to_index(x, y, z), true);

        let criteria = |chunk: &Chunk| {
            !*chunk.is_generated.read() || !*chunk.is_uploaded_to_gpu.read()
        };

        // Load the first tile
        if let Some(chunk) = chunk_manager.get_chunk(x, y, z) {
            if criteria(chunk.as_ref()) {
                return ring;
            }
        }

        while !queue.is_empty() {
            for (x, y, z) in queue.drain(..) {
                if let Some(chunk) = chunk_manager.get_chunk(x, y, z) {
                    if chunk.is_fully_opaque() {
                        continue;
                    }
                }

                for &(x, y, z) in &[
                    (x + 1, y, z),
                    (x - 1, y, z),
                    (x, y, z + 1),
                    (x, y, z - 1),
                    (x, y + 1, z),
                    (x, y - 1, z),
                ] {
                    if is_position_valid(x, y, z) && !is_visited[coords_to_index(x, y, z)] {
                        ring.push((x, y, z));
                        is_visited.set(coords_to_index(x, y, z), true);
                    }
                }
            }

            let mut unloaded_chunks = Vec::new();
            for &(x, y, z) in &ring {
                if y >= 0 && y < 16 {
                    let chunk = chunk_manager.get_chunk(x, y, z).unwrap();
                    if criteria(chunk.as_ref()) {
                        unloaded_chunks.push((x, y, z));
                    }
                }
            }
            if !unloaded_chunks.is_empty() {
                return unloaded_chunks;
            } else {
                queue.extend(ring.iter());
                ring.clear();
            }
        }
        Vec::new()
    }
}

impl<'a> System<'a> for ChunkLoading {
    type SystemData = (
        ReadStorage<'a, Interpolator<PlayerPhysicsState>>,
        Read<'a, Arc<ChunkManager>>,
        Read<'a, TexturePack>,
    );

    fn run(&mut self, data: Self::SystemData) {
        let (
            player_physics_state,
            chunk_manager,
            texture_pack,
        ) = data;

        for player_physics_state in (&player_physics_state).join() {
            let state = player_physics_state.get_latest_state();
            let (c_x, c_y, c_z, _, _, _) = ChunkManager::get_chunk_coords(
                state.position.x as i32,
                state.position.y as i32,
                state.position.z as i32,
            );

            // Remove distant chunk columns and unload their chunks
            {
                if *self.expand_chunks.read() {
                    let mut columns_to_remove = Vec::new();
                    for (&(x, z), column) in chunk_manager.loaded_chunk_columns.read().iter() {
                        for (y, chunk) in column.chunks.iter().enumerate() {
                            let y = y as i32;
                            if abs(x - c_x) > RENDER_DISTANCE ||
                                abs(y - c_y) > RENDER_DISTANCE ||
                                abs(z - c_z) > RENDER_DISTANCE {
                                chunk.unload_from_gpu();
                            }
                        }

                        if abs(x - c_x) > RENDER_DISTANCE + 2 ||
                            abs(z - c_z) > RENDER_DISTANCE + 2 {
                            columns_to_remove.push((x, z));
                        }
                    }
                    for xz in columns_to_remove {
                        if let Some(column) = chunk_manager.remove_chunk_column(&xz) {
                            self.chunk_column_pool.write().push(column);
                        }
                    }
                }
            }

            // Reset chunk columns and send them to the caller (world generation)
            {
                let time_cap = Duration::from_micros(500);
                let before = Instant::now();

                for _ in self.request_chunk_columns_rx.try_iter() {
                    let column = match self.chunk_column_pool.write().pop() {
                        Some(column) => {
                            for chunk in column.chunks.iter() {
                                chunk.reset();
                            }
                            column.heighest_blocks.write().fill(0);
                            *column.has_foliage.write() = false;
                            column
                        },
                        None => {
                            Arc::new(ChunkColumn::new())
                        }
                    };
                    if let Err(err) = self.requested_chunk_column_tx.send(column) {
                        eprintln!("{}", err);
                    }
                    if Instant::now().duration_since(before) >= time_cap {
                        break;
                    }
                }
            }

            // Chunk uploading
            {
                for priority_chunk in self.upload_chunks_rx.try_iter() {
                    self.chunk_upload_priority_queue.push(priority_chunk);
                }
                for _ in 0..CHUNK_UPLOADS_PER_FRAME {
                    if let Some(prioritized_chunk) = self.chunk_upload_priority_queue.pop() {
                        let (c_x, c_y, c_z) = *prioritized_chunk;
                        if let Some(chunk) = chunk_manager.get_chunk(c_x, c_y, c_z) {
                            chunk.upload_to_gpu(&texture_pack);
                            *chunk.is_uploaded_to_gpu.write() = true;
                        }
                    }
                }
            }

            if *self.expand_chunks.read() {
                *self.expand_chunks.write() = false;

                let noise_fn = self.noise_fn;
                let upload_chunks_tx = self.upload_chunks_tx.clone();
                let chunk_manager = Arc::clone(&chunk_manager);
                let expand_chunks = Arc::clone(&self.expand_chunks);
                let request_chunk_columns_tx = self.request_chunk_columns_tx.clone();
                let requested_chunk_column_rx = self.requested_chunk_column_rx.clone();

                self.world_generation_thread_pool.spawn(move || {
                    let new_columns = Self::flood_fill_unloaded_columns(&chunk_manager, c_x, c_z, RENDER_DISTANCE + 2);
                    for _ in 0..new_columns.len() {
                        request_chunk_columns_tx.send(()).unwrap();
                    }

                    let mut unloaded_columns = Vec::new();
                    for (x, z) in new_columns {
                        let column = match requested_chunk_column_rx.recv() {
                            Ok(column) => column,
                            Err(err) => {
                                eprintln!("{}", err);
                                return;
                            }
                        };
                        unloaded_columns.push((x, z, column));
                    }

                    // Terrain generation
                    {
                        let chunk_manager = Arc::clone(&chunk_manager);
                        rayon::scope(move |_s| {
                            let cm = Arc::clone(&chunk_manager);
                            rayon::scope(move |s| {
                                for (x, z, column) in unloaded_columns {
                                    let column = Arc::clone(&column);
                                    let chunk_manager = Arc::clone(&cm);
                                    s.spawn(move |_s| {
                                        // Stone
                                        for y in (0..16).rev() {
                                            let y = 16 * y;
                                            for b_y in 0..16 {
                                                for b_x in 0..16 {
                                                    for b_z in 0..16 {
                                                        let x = 16 * x;
                                                        let z = 16 * z;

                                                        let scale = 90.0;

                                                        // Scale the input for the noise function
                                                        let (xf, yf, zf) = (
                                                            (x + b_x as i32) as f64 / scale,
                                                            (y + b_y as i32) as f64 / (scale / 1.0),
                                                            (z + b_z as i32) as f64 / scale);

                                                        let height = (y + b_y as i32) as f64;
                                                        let noise = noise_fn.get(Point3::from([xf, yf, zf])) * 80.0
                                                            + 64.0 + height * 1.7;

                                                        if noise < 256.0 {
                                                            column.set_block(BlockID::Stone, b_x, y as u32 + b_y, b_z);
                                                        }
                                                    };
                                                }
                                            }
                                        }

                                        // Grass and dirt
                                        for b_x in 0..16 {
                                            for b_z in 0..16 {
                                                let y = column.heighest_blocks.read()[16 * b_z + b_x] as i32;

                                                let chunk_y = (y / 16) as i32;
                                                let block_y = (y % 16) as usize;
                                                column.get_chunk(chunk_y).set_block(BlockID::GrassBlock, b_x as u32, block_y as u32, b_z as u32);

                                                for y in (y - 3)..y {
                                                    let chunk_y = (y / 16) as i32;
                                                    let block_y = (y % 16) as usize;

                                                    let chunk = column.get_chunk(chunk_y);
                                                    if chunk.get_block(b_x as u32, block_y as u32, b_z as u32).is_air() {
                                                        continue;
                                                    }
                                                    chunk.set_block(BlockID::Dirt, b_x as u32, block_y as u32, b_z as u32);
                                                }
                                            }
                                        }

                                        chunk_manager.add_chunk_column((x, z), column);
                                    });
                                }
                            });

                            let chunk_manager = Arc::clone(&chunk_manager);
                            rayon::scope(|_s| {
                                let unfoliated_columns = Self::flood_fill_unfoliated_columns(&chunk_manager, c_x, c_z, RENDER_DISTANCE);
                                for (cx, cz) in unfoliated_columns {
                                    let column = chunk_manager.get_column(cx, cz).unwrap();
                                    *column.has_foliage.write() = true;

                                    // Trees
                                    for (x, z) in compute_tree_placement_in_chunk(
                                            &noise_fn,
                                            (cx * 16) as f64, (cz * 16) as f64
                                        ) {
                                        let (x, z) = (x as usize, z as usize);
                                        let y = column.heighest_blocks.read()[16 * z + x] as i32;

                                        if true {
                                            let x = cx * 16 + x as i32;
                                            let z = cz * 16 + z as i32;

                                            let h = 5;
                                            for i in y + 1..y + 1 + h {
                                                chunk_manager.set_block(BlockID::OakLog, x, i, z);
                                            }

                                            for yy in y + h - 2..=y + h - 1 {
                                                for xx in x - 2..=x + 2 {
                                                    for zz in z - 2..=z + 2 {
                                                        if xx != x || zz != z {
                                                            chunk_manager.set_block(BlockID::OakLeaves, xx, yy, zz);
                                                        }
                                                    }
                                                }
                                            }

                                            for xx in x - 1..=x + 1 {
                                                for zz in z - 1..=z + 1 {
                                                    if xx != x || zz != z {
                                                        chunk_manager.set_block(BlockID::OakLeaves, xx, y + h, zz);
                                                    }
                                                }
                                            }

                                            chunk_manager.set_block(BlockID::OakLeaves, x, y + h + 1, z);
                                            chunk_manager.set_block(BlockID::OakLeaves, x + 1, y + h + 1, z);
                                            chunk_manager.set_block(BlockID::OakLeaves, x - 1, y + h + 1, z);
                                            chunk_manager.set_block(BlockID::OakLeaves, x, y + h + 1, z + 1);
                                            chunk_manager.set_block(BlockID::OakLeaves, x, y + h + 1, z - 1);
                                        }
                                    }
                                }
                            });
                        });
                    }

                    // Chunk face culling & AO
                    let chunk_manager = Arc::clone(&chunk_manager);
                    rayon::scope(move |s| {
                        let new_chunks = Self::flood_fill_chunks(&chunk_manager, c_x, c_y, c_z, RENDER_DISTANCE);
                        for (c_x, c_y, c_z) in new_chunks {
                            let chunk_manager = Arc::clone(&chunk_manager);
                            let send_chunk = upload_chunks_tx.clone();

                            s.spawn(move |_s| {
                                if let Some(chunk) = chunk_manager.get_chunk(c_x, c_y, c_z) {
                                    if chunk.is_empty() {
                                        *chunk.is_generated.write() = true;
                                        *chunk.is_uploaded_to_gpu.write() = true;
                                        return;
                                    }
                                    chunk_manager.update_blocks(c_x, c_y, c_z, BlockIterator::new());
                                    *chunk.is_generated.write() = true;

                                    if let Err(err) = send_chunk.send(PrioritizedItem {
                                        item: (c_x, c_y, c_z),
                                        priority: 0,
                                    }) {
                                        error!("{}", err);
                                    }
                                }
                            });
                        }
                    });
                    *expand_chunks.write() = true;
                });
            }
        }

        // Dirty chunks (changelist)
        let mut changelist_per_chunk: HashMap<(i32, i32, i32), Vec<(i32, u32, u32, u32)>> = HashMap::new();
        for &change in &*chunk_manager.block_changelist.read() {
            for x in -1..=1 {
                for y in -1..=1 {
                    for z in -1..=1 {
                        let (
                            c_x, c_y, c_z,
                            b_x, b_y, b_z,
                        ) = ChunkManager::get_chunk_coords(change.2 + x, change.3 + y, change.4 + z);
                        // change.0 is priority
                        changelist_per_chunk.entry((c_x, c_y, c_z)).or_default().push((change.0, b_x, b_y, b_z));
                    }
                }
            }
        }
        chunk_manager.block_changelist.write().clear();

        for ((c_x, c_y, c_z), dirty_blocks) in changelist_per_chunk {
            let send_chunks = self.upload_chunks_tx.clone();
            let chunk_manager = Arc::clone(&chunk_manager);
            let highest_priority = dirty_blocks.iter().map(|i| i.0).max().unwrap_or(0);
            let thread_pool = if highest_priority == 0 {
                &self.world_generation_thread_pool
            } else {
                &self.player_interaction_thread_pool
            };

            thread_pool.spawn(move || {
                let bxyz = dirty_blocks.iter().map(|i| (i.1, i.2, i.3));

                match chunk_manager.get_chunk(c_x, c_y, c_z) {
                    None => return,
                    Some(chunk) => {
                        chunk_manager.update_blocks(c_x, c_y, c_z, bxyz);

                        if *chunk.is_uploaded_to_gpu.read() {
                            send_chunks.send(PrioritizedItem {
                                item: (c_x, c_y, c_z),
                                priority: highest_priority,
                            }).unwrap();
                        }
                    }
                }
            });
        }
    }
}