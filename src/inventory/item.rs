use crate::chunk::BlockID;
use crate::shapes::centered_unit_cube;
use crate::types::UVMap;
use std::os::raw::c_void;
use nalgebra_glm::{Mat4, vec3, pi};
use crate::constants::{WINDOW_WIDTH, WINDOW_HEIGHT, GUI_SCALING};
use crate::shader_compilation::ShaderProgram;
use nalgebra::Matrix4;

#[derive(Copy, Clone)]
pub struct ItemStack {
    pub item: BlockID,
    pub amount: u32,
    pub(crate) item_render: ItemRender,
}

impl ItemStack {
    pub fn new(amount: u32, block: BlockID, uv_map: &UVMap) -> Self {
        ItemStack {
            item: block,
            amount,
            item_render: ItemRender::new(block, uv_map)
        }
    }
}

#[derive(Copy, Clone)]
pub struct ItemRender {
    vao: u32,
    projection_matrix: Mat4,
}

impl ItemRender {
    pub fn new(block: BlockID, uv_map: &UVMap) -> Self {
        let mut vao = 0;
        gl_call!(gl::CreateVertexArrays(1, &mut vao));

        // Position
        gl_call!(gl::EnableVertexArrayAttrib(vao, 0));
        gl_call!(gl::VertexArrayAttribFormat(vao, 0, 3 as i32, gl::FLOAT, gl::FALSE, 0));
        gl_call!(gl::VertexArrayAttribBinding(vao, 0, 0));

        // Texture coords
        gl_call!(gl::EnableVertexArrayAttrib(vao, 1));
        gl_call!(gl::VertexArrayAttribFormat(vao, 1, 2 as i32, gl::FLOAT, gl::FALSE, 3 * std::mem::size_of::<f32>() as u32));
        gl_call!(gl::VertexArrayAttribBinding(vao, 1, 0));

        // Normals
        gl_call!(gl::EnableVertexArrayAttrib(vao, 2));
        gl_call!(gl::VertexArrayAttribFormat(vao, 2, 3 as i32, gl::FLOAT, gl::FALSE, 5 * std::mem::size_of::<f32>() as u32));
        gl_call!(gl::VertexArrayAttribBinding(vao, 2, 0));

        let mut vbo = 0;
        gl_call!(gl::CreateBuffers(1, &mut vbo));
        let vbo_data = centered_unit_cube(
            -0.5, -0.5, -0.5,
            uv_map.get(&block).unwrap().get_uv_of_every_face());

        gl_call!(gl::NamedBufferData(vbo,
                    (vbo_data.len() * std::mem::size_of::<f32>() as usize) as isize,
                    vbo_data.as_ptr() as *const c_void,
                    gl::STATIC_DRAW));

        gl_call!(gl::VertexArrayVertexBuffer(vao, 0, vbo, 0, (8 * std::mem::size_of::<f32>()) as i32));

        let projection_matrix = nalgebra_glm::ortho(
            0.0, WINDOW_WIDTH as f32, 0.0, WINDOW_HEIGHT as f32, -1000.0, 1000.0);

        ItemRender {
            vao,
            projection_matrix
        }
    }

    pub fn draw(&self, x: f32, y: f32, shader: &mut ShaderProgram) {
        let model_matrix = {
            let translate_matrix = Matrix4::new_translation(&vec3(
                x, y, 1.0));
            let rotate_matrix = {
                let rotate_y = Matrix4::from_euler_angles(0.0, pi::<f32>() / 4.0, 0.0); // 45°
                let rotate_x = Matrix4::from_euler_angles(pi::<f32>() / 6.0, 0.0, 0.0); // 30°
                rotate_x * rotate_y
            };
            let scale_matrix: Mat4 = Matrix4::new_nonuniform_scaling(&(GUI_SCALING * vec3(10.0, 10.0, 10.0)));
            translate_matrix * rotate_matrix * scale_matrix
        };

        shader.use_program();
        shader.set_uniform_matrix4fv("model", model_matrix.as_ptr());
        shader.set_uniform_matrix4fv("projection", self.projection_matrix.as_ptr());
        shader.set_uniform1i("tex", 0);

        gl_call!(gl::BindVertexArray(self.vao));
        gl_call!(gl::DrawArrays(gl::TRIANGLES, 0, 36 as i32));
    }
}