#![feature(entry_insert)]
#[macro_use]
extern crate lazy_static;

extern crate pretty_env_logger;
#[macro_use]
extern crate log;
extern crate specs;

use std::os::raw::c_void;

use glfw::{Action, Context, Key, MouseButton};
use nalgebra::{Matrix4, Vector3};
use nalgebra_glm::{IVec3, Mat4, Vec3};
use nalgebra_glm::vec3;

use crate::aabb::{get_block_aabb, AABB};
use crate::chunk::BlockID;
use crate::chunk_manager::ChunkManager;
use crate::constants::*;
use crate::debugging::*;
use crate::input::InputCache;
use crate::player::{PlayerPhysicsState, PlayerState};
use crate::shader_compilation::ShaderProgram;
use crate::texture_pack::generate_array_texture;
use crate::util::Forward;
use crate::window::create_window;
use crate::gui::{create_crosshair_vao, draw_crosshair, create_gui_icons_texture, create_block_outline_vao, create_widgets_texture, create_hotbar_vao, create_hotbar_selection_vao};
use crate::inventory::Inventory;
use crate::physics::Interpolator;
use timer::Timer;
use crate::particle_system::ParticleSystem;
use std::collections::HashMap;
use crate::fps_counter::FpsCounter;
use std::time::Instant;
use crate::types::UVMap;
use crate::shapes::centered_unit_cube;
use specs::{World, WorldExt, DispatcherBuilder, Builder};
use ecs::components::*;
use ecs::systems::*;
use ecs::resources::*;

#[macro_use]
pub mod debugging;
pub mod draw_commands;
pub mod shader_compilation;
pub mod shapes;
pub mod util;
pub mod chunk_manager;
pub mod chunk;
pub mod raycast;
pub mod block_texture_faces;
pub mod physics;
pub mod aabb;
pub mod constants;
pub mod input;
pub mod window;
pub mod texture_pack;
pub mod player;
pub mod types;
pub mod gui;
pub mod inventory;
pub mod drawing;
pub mod ambient_occlusion;
pub mod timer;
pub mod particle_system;
pub mod fps_counter;
pub mod ecs;

fn main() {
    pretty_env_logger::init();


    let mut world = World::new();
    // world.register::<Position>();
    // world.register::<Velocity>();
    // world.register::<Acceleration>();
    // world.register::<BoundingBox>();
    world.register::<PlayerState>();
    world.register::<Interpolator<PlayerPhysicsState>>();


    let mut dispatcher = DispatcherBuilder::new()
        .with_thread_local({
            let (mut glfw, mut window, events) = create_window(WINDOW_WIDTH, WINDOW_HEIGHT, WINDOW_NAME);

            gl_call!(gl::Enable(gl::DEBUG_OUTPUT));
            gl_call!(gl::Enable(gl::DEBUG_OUTPUT_SYNCHRONOUS));
            gl_call!(gl::DebugMessageCallback(Some(debug_message_callback), 0 as *const c_void));
            gl_call!(gl::DebugMessageControl(gl::DONT_CARE, gl::DONT_CARE, gl::DONT_CARE, 0, 0 as *const u32, gl::TRUE));
            gl_call!(gl::Enable(gl::CULL_FACE));
            gl_call!(gl::CullFace(gl::BACK));
            gl_call!(gl::Enable(gl::DEPTH_TEST));
            gl_call!(gl::Enable(gl::BLEND));
            gl_call!(gl::Viewport(0, 0, WINDOW_WIDTH as i32, WINDOW_HEIGHT as i32));

            ReadWindowEvents {
                glfw,
                window,
                events
            }
        })
        .with(HandlePlayerInput, "handle_player_input", &[])
        .with(UpdatePlayerState, "update_player_state", &["handle_player_input"])
        .with(UpdatePlayerPhysics, "update_player_physics", &["update_player_state"])
        .with(AdvanceGlobalTime, "advance_global_time", &["update_player_physics"])
        .build();


    world.insert(InputCache::default());
    world.insert(Timer::default());
    // world.insert(uv_map);
    world.insert({
        let mut chunk_manager = ChunkManager::new();
        chunk_manager.generate_terrain();
        chunk_manager
    });



    let (item_array_texture, uv_map) = generate_array_texture();
    gl_call!(gl::BindTextureUnit(0, item_array_texture));

    let gui_icons_texture = create_gui_icons_texture();
    gl_call!(gl::ActiveTexture(gl::TEXTURE0 + 1));
    gl_call!(gl::BindTexture(gl::TEXTURE_2D, gui_icons_texture));

    let gui_widgets_texture = create_widgets_texture();
    gl_call!(gl::ActiveTexture(gl::TEXTURE0 + 2));
    gl_call!(gl::BindTexture(gl::TEXTURE_2D, gui_widgets_texture));

    let mut voxel_shader = ShaderProgram::compile("src/shaders/voxel.vert", "src/shaders/voxel.frag");
    let mut gui_shader = ShaderProgram::compile("src/shaders/gui.vert", "src/shaders/gui.frag");
    let mut outline_shader = ShaderProgram::compile("src/shaders/outline.vert", "src/shaders/outline.frag");
    let mut item_shader = ShaderProgram::compile("src/shaders/item.vert", "src/shaders/item.frag");
    let mut particle_shader = ShaderProgram::compile("src/shaders/particle.vert", "src/shaders/particle.frag");
    let mut hand_shader = ShaderProgram::compile("src/shaders/hand.vert", "src/shaders/hand.frag");

    let crosshair_vao = create_crosshair_vao();
    let block_outline_vao = create_block_outline_vao();
    let hotbar_vao = create_hotbar_vao();
    let hotbar_selection_vao = create_hotbar_selection_vao();

    let mut inventory = Inventory::new(&uv_map);

    let mut block_placing_last_executed = Instant::now();

    let player = world.create_entity()
        .with(PlayerState::new())
        .with(Interpolator::new(
            1.0 / PHYSICS_TICKRATE,
            PlayerPhysicsState::new_at_position(vec3(0.0f32, 30.0, 0.0)),
        ))
        .build();

    // let mut player_state = PlayerState::new();
    // let mut player_physics_state = Interpolator::new(
    //     1.0 / PHYSICS_TICKRATE,
    //     PlayerPhysicsState::new_at_position(vec3(0.0f32, 30.0, 0.0)),
    // );

    // let mut chunk_manager = ChunkManager::new();
    // chunk_manager.generate_terrain();
    // chunk_manager.single();
    // chunk_manager.rebuild_dirty_chunks(&uv_map);

    // let mut input_cache = InputCache::default();
    //
    let mut particle_systems: HashMap<&str, ParticleSystem> = HashMap::new();
    particle_systems.insert("block_particles", ParticleSystem::new(500));

    // let mut fps_counter = FpsCounter::new();





    let mut hand_vao = 0;
    gl_call!(gl::CreateVertexArrays(1, &mut hand_vao));

    // Position
    gl_call!(gl::EnableVertexArrayAttrib(hand_vao, 0));
    gl_call!(gl::VertexArrayAttribFormat(hand_vao, 0, 3 as i32, gl::FLOAT, gl::FALSE, 0));
    gl_call!(gl::VertexArrayAttribBinding(hand_vao, 0, 0));

    // Texture coords
    gl_call!(gl::EnableVertexArrayAttrib(hand_vao, 1));
    gl_call!(gl::VertexArrayAttribFormat(hand_vao, 1, 3 as i32, gl::FLOAT, gl::FALSE, 3 * std::mem::size_of::<f32>() as u32));
    gl_call!(gl::VertexArrayAttribBinding(hand_vao, 1, 0));

    // Normals
    gl_call!(gl::EnableVertexArrayAttrib(hand_vao, 2));
    gl_call!(gl::VertexArrayAttribFormat(hand_vao, 2, 3 as i32, gl::FLOAT, gl::FALSE, 6 * std::mem::size_of::<f32>() as u32));
    gl_call!(gl::VertexArrayAttribBinding(hand_vao, 2, 0));

    let mut hand_vbo = 0;
    gl_call!(gl::CreateBuffers(1, &mut hand_vbo));


    gl_call!(gl::VertexArrayVertexBuffer(hand_vao, 0, hand_vbo, 0, (9 * std::mem::size_of::<f32>()) as i32));







    loop {
        dispatcher.dispatch(&world);

        // fps_counter.update();

        // Get looking block coords
        // let targeted_block = {
        //     let is_solid_block_at = |x: i32, y: i32, z: i32| {
        //         chunk_manager.is_solid_block_at(x, y, z)
        //     };
        //
        //     let fw = player_state.rotation.forward();
        //     let player = player_physics_state.get_interpolated_state();
        //     raycast::raycast(
        //         &is_solid_block_at,
        //         &(player.position + vec3(0., *player_state.camera_height.get_interpolated_state(), 0.)),
        //         &fw.normalize(),
        //         REACH_DISTANCE)
        // };

        // glfw.poll_events();
        // for (_, event) in glfw::flush_messages(&events) {
        //     input_cache.handle_event(&event);
        //     inventory.handle_input_event(&event);
        //
        //     match event {
        //         glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
        //             window.set_should_close(true);
        //         }
        //
        //         glfw::WindowEvent::Key(Key::P, _, Action::Press, _) => {
        //             if global_timer.is_paused() {
        //                 global_timer.resume()
        //             } else {
        //                 global_timer.pause();
        //             }
        //         }
        //
        //         glfw::WindowEvent::CursorPos(_, _) => {
        //             player_state.rotate_camera(
        //                 input_cache.cursor_rel_pos.x as f32,
        //                 input_cache.cursor_rel_pos.y as f32);
        //         }
        //
        //         glfw::WindowEvent::MouseButton(button, Action::Press, _) => {
        //             block_placing_last_executed = Instant::now();
        //
        //             match button {
        //                 MouseButton::Button1 => {
        //                     if let &Some(((x, y, z), _)) = &targeted_block {
        //                         let mut particle_system = particle_systems.get_mut("block_particles").unwrap();
        //                         break_block((x, y, z), &mut chunk_manager, &mut particle_system, &uv_map);
        //                     }
        //                 }
        //                 MouseButton::Button2 => {
        //                     if let &Some(((x, y, z), normal)) = &targeted_block {
        //                         place_block((x, y, z), &normal, &player_physics_state.get_latest_state().aabb, &inventory, &mut chunk_manager);
        //                     }
        //                 },
        //                 _ => {}
        //             }
        //         }
        //         _ => {}
        //     }
        //     player_state.handle_input_event(&event);
        //     player_physics_state.get_latest_state().handle_input_event(&event, &mut player_state);
        // }

        // {
        //     let now = Instant::now();
        //     if now.duration_since(block_placing_last_executed).as_secs_f32() >= 0.25 {
        //         if input_cache.is_mouse_button_pressed(glfw::MouseButtonLeft) {
        //             if let &Some(((x, y, z), _)) = &targeted_block {
        //                 let mut particle_system = particle_systems.get_mut("block_particles").unwrap();
        //                 break_block((x, y, z), &mut chunk_manager, &mut particle_system, &uv_map);
        //             }
        //             block_placing_last_executed = Instant::now();
        //         } else if input_cache.is_mouse_button_pressed(glfw::MouseButtonRight) {
        //             if let &Some(((x, y, z), normal)) = &targeted_block {
        //                 place_block((x, y, z), &normal, &player_physics_state.get_latest_state().aabb, &inventory, &mut chunk_manager);
        //             }
        //             block_placing_last_executed = Instant::now();
        //         }
        //     }
        // }

        // player_state.on_update(global_timer.time(), &input_cache, &player_physics_state.get_latest_state());
        // player_physics_state.update_player_physics(global_timer.time(), &input_cache, &chunk_manager, &mut player_state);


        let mut player_state = world.write_component::<PlayerState>();
        let mut player_physics_state = world.write_component::<Interpolator<PlayerPhysicsState>>();

        let mut player_state = player_state.get_mut(player).unwrap();
        let mut player_physics_state = player_physics_state.get_mut(player).unwrap();

        let mut chunk_manager = world.fetch_mut::<ChunkManager>();

        // let mut window = &mut world.fetch_mut::<AppWindow>().window;

        let view_matrix = {
            let player_physics_state = player_physics_state.get_interpolated_state();
            let camera_position = player_physics_state.position + vec3(0., *player_state.camera_height.get_interpolated_state(), 0.);
            let looking_dir = player_state.rotation.forward();
            nalgebra_glm::look_at(&camera_position, &(camera_position + looking_dir), &Vector3::y())
        };

        let projection_matrix = {
            let fov = *player_state.fov.get_interpolated_state();
            nalgebra_glm::perspective(WINDOW_WIDTH as f32 / WINDOW_HEIGHT as f32, fov, NEAR_PLANE, FAR_PLANE)
        };

        // Draw chunks
        {
            chunk_manager.rebuild_dirty_chunks(&uv_map);

            voxel_shader.use_program();
            voxel_shader.set_uniform_matrix4fv("view", view_matrix.as_ptr());
            voxel_shader.set_uniform_matrix4fv("projection", projection_matrix.as_ptr());
            voxel_shader.set_uniform1i("array_texture", 0);

            let (r, g, b, a) = BACKGROUND_COLOR;
            gl_call!(gl::ClearColor(r, g, b, a));
            gl_call!(gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT));
            gl_call!(gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));

            chunk_manager.render_loaded_chunks(&mut voxel_shader);
        }

        // Draw particles
        // {
        //     gl_call!(gl::Disable(gl::CULL_FACE));
        //     particle_shader.use_program();
        //     // particle_shader.set_uniform_matrix4fv("view", view_matrix.as_ptr());
        //     // particle_shader.set_uniform_matrix4fv("projection", projection_matrix.as_ptr());
        //     particle_shader.set_uniform1i("array_texture", 0);
        //
        //     for particle_system in particle_systems.values_mut() {
        //         particle_system.update_all_particles(global_timer.time(), &chunk_manager);
        //         particle_system.render_all_particles(&mut particle_shader, &view_matrix, &projection_matrix);
        //     }
        //     gl_call!(gl::Enable(gl::CULL_FACE));
        // }

        // Block outline
        // if let Some(((x, y, z), _)) = targeted_block {
        //     let (x, y, z) = (x as f32, y as f32, z as f32);
        //     let model_matrix = Matrix4::new_translation(&vec3(x, y, z));
        //
        //     outline_shader.use_program();
        //     outline_shader.set_uniform_matrix4fv("model", model_matrix.as_ptr());
        //     outline_shader.set_uniform_matrix4fv("view", view_matrix.as_ptr());
        //     outline_shader.set_uniform_matrix4fv("projection", projection_matrix.as_ptr());
        //
        //     gl_call!(gl::LineWidth(BLOCK_OUTLINE_WIDTH));
        //     gl_call!(gl::BindVertexArray(block_outline_vao));
        //     gl_call!(gl::DrawArrays(gl::LINES, 0, 24));
        // }

        // Draw hand
        // {
        //     let vbo_data = centered_unit_cube(
        //         -0.5, -0.5, -0.5,
        //         uv_map.get(&inventory.get_selected_item().unwrap()).unwrap().get_uv_of_every_face());
        //
        //     gl_call!(gl::NamedBufferData(hand_vbo,
        //             (vbo_data.len() * std::mem::size_of::<f32>() as usize) as isize,
        //             vbo_data.as_ptr() as *const c_void,
        //             gl::DYNAMIC_DRAW));
        //
        //     let player_pos = player_physics_state.get_interpolated_state().position;
        //     let camera_height = *player_state.camera_height.get_interpolated_state();
        //     let camera_pos = player_pos + vec3(0., camera_height, 0.);
        //
        //     let forward =&player_state.rotation.forward().normalize();
        //     let right = forward.cross(&Vector3::y()).normalize();
        //     let up = right.cross(&forward).normalize();
        //
        //     let model_matrix = {
        //         let translate_matrix = Matrix4::new_translation(&(vec3(
        //             camera_pos.x, camera_pos.y, camera_pos.z) + up * -1.2));
        //
        //         let translate_matrix2 = Matrix4::new_translation(&(vec3(2.0, 0.0, 0.0)));
        //
        //         // dbg!(player_state.rotation);
        //         // let translate_matrix2 = Matrix4::new_translation(&vec3(0.0, 0.0, -2.0));
        //         // let mut rotate_matrix =
        //         // rotate_matrix.m14 = 0.0;
        //         // rotate_matrix.m24 = 0.0;
        //         // rotate_matrix.m34 = 0.0;
        //         // rotate_matrix.m44 = 1.0;
        //         // rotate_matrix.m22 = -1.0;
        //
        //
        //         let rotate_matrix = nalgebra_glm::rotation(-player_state.rotation.y, &vec3(0.0, 1.0, 0.0));
        //         let rotate_matrix = nalgebra_glm::rotation(player_state.rotation.x, &right) * rotate_matrix;
        //
        //         let rotate_matrix = nalgebra_glm::rotation(-35.0f32.to_radians(),&up) * rotate_matrix;
        //
        //
        //         // let rotate_matrix2 = Matrix4::from_euler_angles(
        //         //     -player_state.rotation.x,
        //         //     0.0,
        //         //     0.0,
        //         // );
        //         // let scale_matrix: Mat4 = Matrix4::new_nonuniform_scaling(&vec3(1.0f32, 1.0f32, 1.0f32));
        //         // translate_matrix * rotate_matrix * scale_matrix
        //         translate_matrix * rotate_matrix * translate_matrix2
        //     };
        //
        //     let projection_matrix = {
        //         let fov = 1.22173;
        //         nalgebra_glm::perspective(WINDOW_WIDTH as f32 / WINDOW_HEIGHT as f32, fov, NEAR_PLANE, FAR_PLANE)
        //     };
        //
        //     // let mut model_view: Mat4 = view_matrix * model_matrix;
        //     // model_view.m11 = 1.0;
        //     // model_view.m12 = 0.;
        //     // model_view.m13 = 0.;
        //     //
        //     // model_view.m21 = 0.;
        //     // model_view.m22 = 1.0;
        //     // model_view.m23 = 0.;
        //     //
        //     // model_view.m31 = 0.;
        //     // model_view.m32 = 0.;
        //     // model_view.m33 = 1.0;
        //
        //     hand_shader.use_program();
        //     hand_shader.set_uniform_matrix4fv("model", model_matrix.as_ptr());
        //     hand_shader.set_uniform_matrix4fv("view", view_matrix.as_ptr());
        //     // hand_shader.set_uniform_matrix4fv("model_view", model_view.as_ptr());
        //     hand_shader.set_uniform_matrix4fv("projection", projection_matrix.as_ptr());
        //     hand_shader.set_uniform1i("tex", 0);
        //
        //     gl_call!(gl::BindVertexArray(hand_vao));
        //
        //     gl_call!(gl::Disable(gl::DEPTH_TEST));
        //     gl_call!(gl::DrawArrays(gl::TRIANGLES, 0, 36 as i32));
        //     gl_call!(gl::Enable(gl::DEPTH_TEST));
        // }

        // Draw GUI
        {
            draw_crosshair(crosshair_vao, &mut gui_shader);
            gl_call!(gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
            gl_call!(gl::Disable(gl::DEPTH_TEST));
            inventory.draw_hotbar(hotbar_vao, &mut gui_shader);
            inventory.draw_hotbar_selection_box(hotbar_selection_vao, &mut gui_shader);
            inventory.draw_hotbar_items(&mut item_shader);
            gl_call!(gl::Enable(gl::DEPTH_TEST));
        }

        // window.swap_buffers();
    }
}

fn break_block((x, y, z): (i32, i32, i32), chunk_manager: &mut ChunkManager, particle_system: &mut ParticleSystem, uv_map: &UVMap) {
    let block = chunk_manager.get_block(x, y, z).unwrap();
    chunk_manager.set_block(BlockID::Air, x, y, z);
    particle_system.spawn_block_breaking_particles(vec3(x as f32, y as f32, z as f32), &uv_map, block);
    info!("Destroyed block at ({} {} {})", x, y, z);
}

fn place_block((x, y, z): (i32, i32, i32), normal: &IVec3, player_aabb: &AABB, inventory: &Inventory, chunk_manager: &mut ChunkManager) {
    let adjacent_block = IVec3::new(x, y, z) + normal;
    let adjacent_block_aabb = get_block_aabb(&vec3(
        adjacent_block.x as f32,
        adjacent_block.y as f32,
        adjacent_block.z as f32));
    if !player_aabb.intersects(&adjacent_block_aabb) {
        if let Some(block) = inventory.get_selected_item() {
            chunk_manager.set_block(block, adjacent_block.x, adjacent_block.y, adjacent_block.z);
        }
        info!("Put block at ({} {} {})", adjacent_block.x, adjacent_block.y, adjacent_block.z);
    }
}