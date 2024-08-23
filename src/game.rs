use std::rc::Rc;

use self::red_hat_boy_states::*;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use gloo_utils::format::JsValueSerdeExt;
use rand::{thread_rng, Rng};
use web_sys::HtmlImageElement;

use crate::{
    browser,
    engine::{
        self, Audio, Cell, Game, Image, KeyState, Point, Rect, Renderer, Sheet, Sound, SpriteSheet,
    },
    segment::{platform_and_stone, stone_and_platform},
};

const HEIGHT: i16 = 600;
const TIMELINE_MINIMUM: i16 = 1000;
const OBSTACLE_BUFFER: i16 = 20;

// 課題:
// game.rsとsegment.rsの間に循環依存がある
// segmentが依存するgame内のものを全て別のモジュールに移して、
// segmentとgameからの依存をここに向けることでこれを解決する

fn rightmost(obstacle_list: &Vec<Box<dyn Obstacle>>) -> i16 {
    obstacle_list
        .iter()
        .map(|obstacle| obstacle.right())
        .max_by(|x, y| x.cmp(&y))
        .unwrap_or(0)
}

pub trait Obstacle {
    fn check_intersection(&self, boy: &mut RedHatBoy);
    fn draw(&self, rendere: &Renderer);
    fn move_horizontally(&mut self, x: i16);
    fn right(&self) -> i16;
}

pub struct Platform {
    sheet: Rc<SpriteSheet>,
    bounding_boxes: Vec<Rect>,
    sprites: Vec<Cell>,
    position: Point,
}

impl Platform {
    pub fn new(
        sheet: Rc<SpriteSheet>,
        position: Point,
        sprite_names: &[&str],
        bounding_boxes: &[Rect],
    ) -> Self {
        let sprites = sprite_names
            .iter()
            .filter_map(|sprite_name| sheet.cell(sprite_name).cloned())
            .collect();

        let bounding_boxes = bounding_boxes
            .iter()
            .map(|bounding_box| {
                Rect::new_from_x_y(
                    // todo: なんでここ x() を足してるんだろう
                    // 相対位置？
                    bounding_box.x() + position.x,
                    bounding_box.y() + position.y,
                    bounding_box.width,
                    bounding_box.height,
                )
            })
            .collect();

        Platform {
            sheet,
            position,
            sprites,
            bounding_boxes,
        }
    }

    fn bounding_boxes(&self) -> &Vec<Rect> {
        &self.bounding_boxes
    }
}

impl Obstacle for Platform {
    fn draw(&self, renderer: &Renderer) {
        // この実装では、obstacleは複数行にわたらず、ぴったり一行に連なることを想定している（オフセットの取り方的に）
        let mut x = 0;
        self.sprites.iter().for_each(|sprite| {
            self.sheet.draw(
                renderer,
                &Rect::new_from_x_y(
                    sprite.frame.x,
                    sprite.frame.y,
                    sprite.frame.w,
                    sprite.frame.h,
                ),
                // Just use position and the standard width in the tilesheet
                &Rect::new_from_x_y(
                    self.position.x + x, // 総体としての position に、オフセット x を加える
                    self.position.y,
                    sprite.frame.w,
                    sprite.frame.h,
                ),
            );
            x += sprite.frame.w;
        });

        // debug用にbounding boxを描画
        for bbox in self.bounding_boxes() {
            renderer.draw_rect(&bbox)
        }
    }

    fn move_horizontally(&mut self, x: i16) {
        self.position.x += x;
        self.bounding_boxes.iter_mut().for_each(|bounding_box| {
            bounding_box.set_x(bounding_box.position.x + x);
        })
    }

    fn check_intersection(&self, boy: &mut RedHatBoy) {
        if let Some(box_to_land_on) = self
            .bounding_boxes()
            .iter()
            .find(|&bounding_box| boy.bounding_box().intersects(bounding_box))
        {
            // y座標が大きいほど低いことに注意
            if boy.velocity_y() > 0 && boy.pos_y() < self.position.y {
                boy.land_on(box_to_land_on.y());
            } else {
                boy.knock_out();
            }
        }
    }

    fn right(&self) -> i16 {
        self.bounding_boxes()
            .last()
            .unwrap_or(&Rect::default())
            .right()
    }
}

pub struct Barrier {
    image: Image,
}

impl Barrier {
    pub fn new(image: Image) -> Self {
        Barrier { image }
    }
}

impl Obstacle for Barrier {
    fn check_intersection(&self, boy: &mut RedHatBoy) {
        if boy.bounding_box().intersects(self.image.bounding_box()) {
            boy.knock_out()
        }
    }

    fn draw(&self, renderer: &Renderer) {
        self.image.draw(renderer)
    }

    fn move_horizontally(&mut self, x: i16) {
        self.image.move_horizontally(x);
    }

    fn right(&self) -> i16 {
        self.image.right()
    }
}

pub enum WalkTheDog {
    Loading,
    Loaded(Walk),
}

impl WalkTheDog {
    pub fn new() -> Self {
        WalkTheDog::Loading
    }
}

#[async_trait(?Send)]
impl Game for WalkTheDog {
    async fn initialize(&self) -> Result<Box<dyn Game>> {
        match self {
            WalkTheDog::Loading => {
                let json = browser::fetch_json("rhb.json").await?;
                let background = engine::load_image("BG.png").await?;
                let stone = engine::load_image("Stone.png").await?;

                let tiles = browser::fetch_json("tiles.json").await?;
                let sprite_sheet = Rc::new(SpriteSheet::new(
                    tiles.into_serde()?,
                    engine::load_image("tiles.png").await?,
                ));

                //let audio = Audio::new()?;
                //let sound = audio.load_sound("SFX_Jump_23.mp3").await?;
                let rhb = RedHatBoy::new(
                    json.into_serde::<Sheet>()?,
                    engine::load_image("rhb.png").await?,
                );

                let background_width = background.width() as i16;
                let starting_obstacles = stone_and_platform(stone.clone(), sprite_sheet.clone(), 0);
                let timeline = rightmost(&starting_obstacles);
                Ok(Box::new(WalkTheDog::Loaded(Walk {
                    boy: rhb,
                    backgrounds: [
                        Image::new(background.clone(), Point { x: 0, y: 0 }),
                        Image::new(
                            background,
                            Point {
                                x: background_width,
                                y: 0,
                            },
                        ),
                    ],
                    // FLOORが600で、岩の高さが54ピクセル
                    obstacles: starting_obstacles,
                    obstacle_sheet: sprite_sheet,
                    stone,
                    timeline,
                })))
            }
            WalkTheDog::Loaded(_) => Err(anyhow!("Error: Game is already initialized!")),
        }
    }

    fn update(&mut self, keystate: &KeyState) {
        if let WalkTheDog::Loaded(walk) = self {
            if keystate.is_pressed("ArrowDown") {
                walk.boy.slide()
            }
            if keystate.is_pressed("ArrowRight") {
                walk.boy.run_right()
            }
            if keystate.is_pressed("Space") {
                walk.boy.jump();
            }
            walk.boy.update();

            let velocity = walk.velocity();
            let [bg_fst, bg_snd] = &mut walk.backgrounds;
            bg_fst.move_horizontally(velocity);
            bg_snd.move_horizontally(velocity);

            if bg_fst.right() < 0 {
                bg_fst.set_x(bg_snd.right());
            }
            if bg_snd.right() < 0 {
                bg_snd.set_x(bg_fst.right());
            }

            walk.obstacles.retain(|obstacle| obstacle.right() > 0);

            walk.obstacles.iter_mut().for_each(|obstacle| {
                obstacle.move_horizontally(velocity);
                obstacle.check_intersection(&mut walk.boy);
            });

            if walk.timeline < TIMELINE_MINIMUM {
                walk.generate_next_segment()
            } else {
                walk.timeline += velocity;
            }
        }
    }

    fn draw(&self, renderer: &Renderer) {
        renderer.clear(&Rect::new_from_x_y(0, 0, 600, 600));

        if let WalkTheDog::Loaded(walk) = self {
            walk.backgrounds.iter().for_each(|background| {
                background.draw(renderer);
            });
            walk.boy.draw(renderer);

            walk.obstacles
                .iter()
                .for_each(|obstacle| obstacle.draw(renderer));
        }
    }
}

pub struct Walk {
    obstacle_sheet: Rc<SpriteSheet>,
    boy: RedHatBoy,
    backgrounds: [Image; 2],
    obstacles: Vec<Box<dyn Obstacle>>,
    stone: HtmlImageElement,
    timeline: i16,
}

impl Walk {
    fn velocity(&self) -> i16 {
        -self.boy.walking_speed()
    }

    fn generate_next_segment(&mut self) {
        let mut rng = thread_rng();
        let next_segment = rng.gen_range(0..2);

        let mut next_obstacles = match next_segment {
            0 => stone_and_platform(
                self.stone.clone(),
                self.obstacle_sheet.clone(),
                self.timeline + OBSTACLE_BUFFER,
            ),
            1 => platform_and_stone(
                self.stone.clone(),
                self.obstacle_sheet.clone(),
                self.timeline + OBSTACLE_BUFFER,
            ),
            _ => vec![],
        };

        self.timeline = rightmost(&next_obstacles);
        self.obstacles.append(&mut next_obstacles);
    }
}

pub struct RedHatBoy {
    state_machine: RedHatBoyStateMachine,
    sprite_sheet: Sheet,
    image: HtmlImageElement,
}

impl RedHatBoy {
    fn new(sheet: Sheet, image: HtmlImageElement) -> Self {
        RedHatBoy {
            state_machine: RedHatBoyStateMachine::Idle(RedHatBoyState::new()),
            sprite_sheet: sheet,
            image,
        }
    }

    fn frame_name(&self) -> String {
        format!(
            "{} ({}).png",
            self.state_machine.frame_name(),
            (self.state_machine.context().frame / 3) + 1
        )
    }

    fn walking_speed(&self) -> i16 {
        self.state_machine.context().velocity.x
    }

    fn current_sprite(&self) -> Option<&Cell> {
        self.sprite_sheet.frames.get(&self.frame_name())
    }

    fn destination_box(&self) -> Rect {
        let sprite = self.current_sprite().expect("Cell not found");

        Rect::new_from_x_y(
            (self.state_machine.context().position.x + sprite.sprite_source_size.x as i16).into(),
            (self.state_machine.context().position.y + sprite.sprite_source_size.y as i16).into(),
            sprite.frame.w.into(),
            sprite.frame.h.into(),
        )
    }

    fn bounding_box(&self) -> Rect {
        const X_OFFSET: i16 = 18;
        const Y_OFFSET: i16 = 14;
        const WIDTH_OFFSET: i16 = 28;

        Rect::new_from_x_y(
            self.destination_box().x() + X_OFFSET,
            self.destination_box().y() + Y_OFFSET,
            self.destination_box().width - WIDTH_OFFSET,
            self.destination_box().height - Y_OFFSET,
        )
    }

    fn draw(&self, renderer: &Renderer) {
        let sprite = self.current_sprite().expect("Cell not found");

        renderer.draw_image(
            &self.image,
            &Rect::new_from_x_y(
                sprite.frame.x.into(),
                sprite.frame.y.into(),
                sprite.frame.w.into(),
                sprite.frame.h.into(),
            ),
            &Rect::new_from_x_y(
                (self.state_machine.context().position.x + sprite.sprite_source_size.x as i16)
                    .into(),
                (self.state_machine.context().position.y + sprite.sprite_source_size.y as i16)
                    .into(),
                sprite.frame.w.into(),
                sprite.frame.h.into(),
            ),
        );

        // debug用
        renderer.draw_rect(&self.bounding_box())
    }

    fn update(&mut self) {
        self.state_machine = self.state_machine.clone().update();
    }

    fn run_right(&mut self) {
        self.state_machine = self.state_machine.clone().transition(Event::Run);
    }

    fn slide(&mut self) {
        self.state_machine = self.state_machine.clone().transition(Event::Slide);
    }

    fn jump(&mut self) {
        self.state_machine = self.state_machine.clone().transition(Event::Jump);
    }

    fn knock_out(&mut self) {
        self.state_machine = self.state_machine.clone().transition(Event::KnockOut);
    }

    fn land_on(&mut self, pos: i16) {
        self.state_machine = self.state_machine.clone().transition(Event::Land(pos));
    }

    fn pos_y(&self) -> i16 {
        self.state_machine.context().position.y
    }

    fn velocity_y(&self) -> i16 {
        self.state_machine.context().velocity.y
    }
}

#[derive(Clone, Copy)]
enum RedHatBoyStateMachine {
    Idle(RedHatBoyState<Idle>),
    Running(RedHatBoyState<Running>),
    Sliding(RedHatBoyState<Sliding>),
    Jumping(RedHatBoyState<Jumping>),
    Falling(RedHatBoyState<Falling>),
    KnockedOut(RedHatBoyState<KnockedOut>),
}

pub enum Event {
    Run,
    Jump,
    Slide,
    KnockOut,
    Update,
    Land(i16),
}

impl RedHatBoyStateMachine {
    // 止まってる時もジャンプできるようにするのが自然？あとでやってみよう
    fn transition(self, event: Event) -> Self {
        match (self.clone(), event) {
            (RedHatBoyStateMachine::Idle(state), Event::Run) => state.run().into(),
            (RedHatBoyStateMachine::Idle(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Running(state), Event::Slide) => state.slide().into(),
            (RedHatBoyStateMachine::Running(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Running(state), Event::Jump) => state.jump().into(),
            (RedHatBoyStateMachine::Running(state), Event::KnockOut) => state.knock_out().into(),
            (RedHatBoyStateMachine::Running(state), Event::Land(pos)) => state.land_on(pos).into(),
            (RedHatBoyStateMachine::Jumping(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Jumping(state), Event::Land(pos)) => state.land_on(pos).into(),
            (RedHatBoyStateMachine::Jumping(state), Event::KnockOut) => state.knock_out().into(),
            (RedHatBoyStateMachine::Sliding(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Sliding(state), Event::KnockOut) => state.knock_out().into(),
            (RedHatBoyStateMachine::Sliding(state), Event::Land(pos)) => state.land_on(pos).into(),
            (RedHatBoyStateMachine::Falling(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::KnockedOut(state), Event::Update) => state.update().into(),
            _ => self,
        }
    }

    fn frame_name(&self) -> &str {
        match self {
            RedHatBoyStateMachine::Idle(state) => state.frame_name(),
            RedHatBoyStateMachine::Running(state) => state.frame_name(),
            RedHatBoyStateMachine::Sliding(state) => state.frame_name(),
            RedHatBoyStateMachine::Jumping(state) => state.frame_name(),
            RedHatBoyStateMachine::Falling(state) => state.frame_name(),
            RedHatBoyStateMachine::KnockedOut(state) => state.frame_name(),
        }
    }

    fn context(&self) -> &RedHatBoyContext {
        match self {
            RedHatBoyStateMachine::Idle(state) => &state.context(),
            RedHatBoyStateMachine::Running(state) => &state.context(),
            RedHatBoyStateMachine::Sliding(state) => &state.context(),
            RedHatBoyStateMachine::Jumping(state) => &state.context(),
            RedHatBoyStateMachine::Falling(state) => &state.context(),
            RedHatBoyStateMachine::KnockedOut(state) => &state.context(),
        }
    }

    fn update(self) -> Self {
        self.transition(Event::Update)
    }
}

impl From<RedHatBoyState<Idle>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Idle>) -> Self {
        RedHatBoyStateMachine::Idle(state)
    }
}

impl From<RedHatBoyState<Running>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Running>) -> Self {
        RedHatBoyStateMachine::Running(state)
    }
}

impl From<RedHatBoyState<Sliding>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Sliding>) -> Self {
        RedHatBoyStateMachine::Sliding(state)
    }
}

impl From<RedHatBoyState<Jumping>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Jumping>) -> Self {
        RedHatBoyStateMachine::Jumping(state)
    }
}

impl From<RedHatBoyState<Falling>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Falling>) -> Self {
        RedHatBoyStateMachine::Falling(state)
    }
}

impl From<RedHatBoyState<KnockedOut>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<KnockedOut>) -> Self {
        RedHatBoyStateMachine::KnockedOut(state)
    }
}

mod red_hat_boy_states {
    use super::HEIGHT;
    use crate::engine::{Audio, Point, Sound};

    use super::RedHatBoyStateMachine;
    const FLOOR: i16 = 479;
    const PLAYER_HEIGHT: i16 = HEIGHT - FLOOR;
    const STARTING_POINT: i16 = -20;
    const IDLE_FRAME_NAME: &str = "Idle";
    const SLIDING_NAME: &str = "Slide";
    const RUN_FRAME_NAME: &str = "Run";
    const JUMP_FRAME_NAME: &str = "Jump";
    const FALL_FRAME_NAME: &str = "Dead";

    const IDLE_FRAMES: u8 = 29;
    const SLIDING_FRAMES: u8 = 14;
    const RUNNING_FRAMES: u8 = 23;
    const JUMP_FRAMES: u8 = 35;
    const FALL_FRAMES: u8 = 29;

    const RUNNING_SPEED: i16 = 4;
    const JUMP_SPEED: i16 = -23;

    const GRAVITY: i16 = 1;
    const TERMINAL_VELOCITY: i16 = 20;

    #[derive(Clone, Copy)]
    pub struct Idle;

    #[derive(Clone, Copy)]
    pub struct Running;

    #[derive(Clone, Copy)]
    pub struct Sliding;

    #[derive(Clone, Copy)]
    pub struct Jumping;

    #[derive(Clone, Copy)]
    pub struct Falling;

    #[derive(Clone, Copy)]
    pub struct KnockedOut;

    #[derive(Clone, Copy)]
    pub struct RedHatBoyState<S> {
        context: RedHatBoyContext,
        _state: S,
    }

    impl<S> RedHatBoyState<S> {
        pub fn context(&self) -> &RedHatBoyContext {
            &self.context
        }
    }

    // これがタイプステートパターンなのかな？
    // すごい、Idleの部分が値みたいな直観があるせいで、依存型に見える
    impl RedHatBoyState<Idle> {
        pub fn new() -> Self {
            RedHatBoyState {
                context: RedHatBoyContext {
                    frame: 0,
                    position: Point {
                        x: STARTING_POINT,
                        y: FLOOR,
                    },
                    velocity: Point { x: 0, y: 0 },
                },
                _state: Idle {},
            }
        }

        pub fn run(self) -> RedHatBoyState<Running> {
            RedHatBoyState {
                context: self.context.reset_frame().run_right(),
                _state: Running {},
            }
        }

        pub fn frame_name(&self) -> &str {
            IDLE_FRAME_NAME
        }

        pub fn update(mut self) -> Self {
            self.context = self.context.update(IDLE_FRAMES);
            self
        }
    }

    impl RedHatBoyState<Running> {
        pub fn frame_name(&self) -> &str {
            RUN_FRAME_NAME
        }

        pub fn update(mut self) -> Self {
            self.context = self.context.update(RUNNING_FRAMES);
            self
        }

        pub fn slide(self) -> RedHatBoyState<Sliding> {
            RedHatBoyState {
                context: self.context.reset_frame(),
                _state: Sliding {},
            }
        }

        pub fn jump(self) -> RedHatBoyState<Jumping> {
            RedHatBoyState {
                context: self.context.set_vertical_velocity(JUMP_SPEED).reset_frame(),
                //.play_jump_sound(),
                _state: Jumping {},
            }
        }

        pub fn knock_out(self) -> RedHatBoyState<Falling> {
            RedHatBoyState {
                context: self.context.reset_frame().stop(),
                _state: Falling {},
            }
        }

        pub fn land_on(self, pos: i16) -> RedHatBoyState<Running> {
            RedHatBoyState {
                // running -> running だから frame resetはしちゃだめ
                context: self.context.set_on(pos as i16),
                _state: Running,
            }
        }
    }

    impl RedHatBoyState<Sliding> {
        pub fn frame_name(&self) -> &str {
            SLIDING_NAME
        }

        pub fn update(mut self) -> SlidingEndState {
            self.context = self.context.update(SLIDING_FRAMES);

            if self.context.frame >= SLIDING_FRAMES {
                SlidingEndState::Complete(self.stand())
            } else {
                SlidingEndState::Sliding(self)
            }
        }

        pub fn stand(self) -> RedHatBoyState<Running> {
            RedHatBoyState {
                context: self.context.reset_frame(),
                _state: Running,
            }
        }

        pub fn knock_out(self) -> RedHatBoyState<Falling> {
            RedHatBoyState {
                context: self.context.reset_frame().stop(),
                _state: Falling {},
            }
        }

        pub fn land_on(self, pos: i16) -> RedHatBoyState<Sliding> {
            RedHatBoyState {
                context: self.context.set_on(pos),
                _state: Sliding,
            }
        }
    }

    pub enum SlidingEndState {
        Complete(RedHatBoyState<Running>),
        Sliding(RedHatBoyState<Sliding>),
    }

    impl From<SlidingEndState> for RedHatBoyStateMachine {
        fn from(end_state: SlidingEndState) -> Self {
            match end_state {
                SlidingEndState::Complete(running_state) => running_state.into(),
                SlidingEndState::Sliding(sliding_state) => sliding_state.into(),
            }
        }
    }

    impl RedHatBoyState<Jumping> {
        pub fn frame_name(&self) -> &str {
            JUMP_FRAME_NAME
        }

        pub fn update(mut self) -> JumpingEndState {
            self.context = self.context.update(JUMP_FRAMES);

            // これの閾値を JUMPING FRAMEでやろうとすると空中ジャンプする
            // 地面につくまでの時間（必要フレーム数）は、初速度と重力に依存するので、
            // frame数が毎回それに合致しているとは限らない
            if self.context.position.y >= FLOOR {
                JumpingEndState::Complete(self.land_on(HEIGHT.into()))
            } else {
                JumpingEndState::Jumping(self)
            }
        }

        pub fn land_on(self, pos: i16) -> RedHatBoyState<Running> {
            RedHatBoyState {
                context: self.context.reset_frame().set_on(pos),
                _state: Running,
            }
        }

        pub fn knock_out(self) -> RedHatBoyState<Falling> {
            RedHatBoyState {
                context: self.context.reset_frame().stop(),
                _state: Falling {},
            }
        }
    }

    pub enum JumpingEndState {
        Complete(RedHatBoyState<Running>),
        Jumping(RedHatBoyState<Jumping>),
    }

    impl From<JumpingEndState> for RedHatBoyStateMachine {
        fn from(end_state: JumpingEndState) -> Self {
            match end_state {
                JumpingEndState::Complete(running_state) => running_state.into(),
                JumpingEndState::Jumping(jumping_state) => jumping_state.into(),
            }
        }
    }

    impl RedHatBoyState<Falling> {
        pub fn frame_name(&self) -> &str {
            FALL_FRAME_NAME
        }

        pub fn update(mut self) -> FallingEndState {
            self.context = self.context.update(FALL_FRAMES);

            if self.context.frame >= FALL_FRAMES {
                FallingEndState::Complete(self.knocked_out())
            } else {
                FallingEndState::Falling(self)
            }
        }

        fn knocked_out(self) -> RedHatBoyState<KnockedOut> {
            RedHatBoyState {
                context: self.context,
                _state: KnockedOut,
            }
        }
    }

    pub enum FallingEndState {
        Complete(RedHatBoyState<KnockedOut>),
        Falling(RedHatBoyState<Falling>),
    }

    impl From<FallingEndState> for RedHatBoyStateMachine {
        fn from(state: FallingEndState) -> Self {
            match state {
                FallingEndState::Complete(state) => state.into(),
                FallingEndState::Falling(state) => state.into(),
            }
        }
    }

    impl RedHatBoyState<KnockedOut> {
        pub fn frame_name(&self) -> &str {
            FALL_FRAME_NAME
        }

        pub fn update(mut self) -> Self {
            self.context = self.context.update_with_fixed_frame();
            self
        }
    }

    #[derive(Clone, Copy)]
    pub struct RedHatBoyContext {
        pub frame: u8,
        pub position: Point,
        pub velocity: Point,
    }

    impl RedHatBoyContext {
        pub fn update(mut self, frame_count: u8) -> Self {
            if self.velocity.y < TERMINAL_VELOCITY {
                self.velocity.y += GRAVITY;
            }
            //log!("Gravity {}", self.velocity.y);
            if self.frame < frame_count {
                self.frame += 1;
            } else {
                self.frame = 0;
            }

            self.position.y += self.velocity.y;

            if self.position.y > FLOOR {
                // ここで速度も0にしてもいいかも
                // 今速度を着地後も適当にしているのは、状態が変わって、状態が変わるとcontextがリセットされて問題ないからか
                self.position.y = FLOOR;
            }

            self
        }

        // ch05最後の演習問題の解答でオリジナルなので、後々整合性が取れなくなったらまずここを疑う
        pub fn update_with_fixed_frame(mut self) -> Self {
            if self.velocity.y < TERMINAL_VELOCITY {
                self.velocity.y += GRAVITY;
            }
            //log!("Gravity {}", self.velocity.y);

            self.position.y += self.velocity.y;

            if self.position.y > FLOOR {
                // ここで速度も0にしてもいいかも
                // 今速度を着地後も適当にしているのは、状態が変わって、状態が変わるとcontextがリセットされて問題ないからか
                self.position.y = FLOOR;
            }

            self
        }

        fn reset_frame(mut self) -> Self {
            self.frame = 0;
            self
        }

        pub fn run_right(mut self) -> Self {
            self.velocity.x += RUNNING_SPEED;
            self
        }

        fn set_vertical_velocity(mut self, y: i16) -> Self {
            self.velocity.y = y;
            self
        }

        fn stop(mut self) -> Self {
            self.velocity.x = 0;
            self
        }

        fn set_on(mut self, position: i16) -> Self {
            let position = position - PLAYER_HEIGHT;
            self.position.y = position;
            self
        }

        //fn play_jump_sound(self) -> Self {
        //    if let Err(err) = self.audio.play_sound(&self.jump_sound) {
        //        log!("Error playing jump sound {:#?}", err);
        //    }
        //    self
        //}
    }
}
