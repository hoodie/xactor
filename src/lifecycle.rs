use futures::{
    channel::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    FutureExt, StreamExt,
};

use crate::{
    addr::ActorEvent,
    runtime::spawn,
    Result, {Actor, Addr, Context},
};

pub struct LifeCycle<A: Actor> {
    ctx: Context<A>,
    tx: std::sync::Arc<UnboundedSender<ActorEvent<A>>>,
    rx: UnboundedReceiver<ActorEvent<A>>,
    tx_exit: oneshot::Sender<()>,
}

impl<A: Actor> Default for LifeCycle<A> {
    fn default() -> Self {
        let (tx_exit, rx_exit) = oneshot::channel();
        let rx_exit = rx_exit.shared();
        let (ctx, rx, tx) = Context::new(Some(rx_exit));
        Self {
            ctx,
            rx,
            tx,
            tx_exit,
        }
    }
}

impl<A: Actor> LifeCycle<A> {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn address(&self) -> Addr<A> {
        self.ctx.address()
    }

    pub async fn start(self, mut actor: A) -> Result<Addr<A>> {
        let Self {
            mut ctx,
            mut rx,
            tx,
            tx_exit,
        } = self;

        let rx_exit = ctx.rx_exit.clone();
        let actor_id = ctx.actor_id();

        // Call started
        actor.started(&mut ctx).await?;

        spawn({
            async move {
                while let Some(event) = rx.next().await {
                    match event {
                        ActorEvent::Exec(f) => f(&mut actor, &mut ctx).await,
                        ActorEvent::Stop(_err) => break,
                        ActorEvent::RemoveStream(id) => {
                            if ctx.streams.contains(id) {
                                ctx.streams.remove(id);
                            }
                        }
                    }
                }

                actor.stopped(&mut ctx).await;

                ctx.abort_streams();
                ctx.abort_intervals();

                tx_exit.send(()).ok();
            }
        });

        Ok(Addr {
            actor_id,
            tx,
            rx_exit,
        })
    }

    pub async fn start_supervised<F>(self, f: F) -> Result<Addr<A>>
    where
        F: Fn() -> A + Send + 'static,
    {
        let Self {
            mut ctx,
            mut rx,
            tx,
            ..
        } = self;

        let rx_exit = ctx.rx_exit.clone();
        let actor_id = ctx.actor_id();

        // Create the actor
        let mut actor = f();

        // let (tx_exit_supervisor, rx_exit_supervisor) = oneshot::channel();
        let supervisor_addr = Addr {
            actor_id,
            tx,
            // rx_exit: Some(rx_exit_supervisor.shared()),
            rx_exit
        };

        // Call started
        actor.started(&mut ctx).await?; // FIXME: needs it's own context

        spawn({
            async move {
                //println!("<supervisor>");
                'restart_loop: loop {
                    //println!("  <restart_loop>");
                    'event_loop: loop {
                        //println!("    <event_loop>");
                        match rx.next().await {
                            None => {
                                println!("   ✋ break restart_loop");
                                break 'restart_loop;
                            }
                            Some(ActorEvent::Stop(_err)) => {
                                println!("   ✋ break event_loop");
                                break 'event_loop;
                            }
                            Some(ActorEvent::Exec(f)) => f(&mut actor, &mut ctx).await,
                            Some(ActorEvent::RemoveStream(id)) => {
                                if ctx.streams.contains(id) {
                                    ctx.streams.remove(id);
                                }
                            }
                        }
                        //println!("    </event_loop>");
                    }

                    actor.stopped(&mut ctx).await;
                    ctx.abort_streams();
                    ctx.abort_intervals();

                    actor = f();
                    actor.started(&mut ctx).await.ok();
                    // println!("  </restart_loop>");
                }
                actor.stopped(&mut ctx).await;
                ctx.abort_streams();
                ctx.abort_intervals();
                // tx_exit_supervisor.send(()).ok();
                // println!("</supervisor>");
            }
        });

        Ok(supervisor_addr)
    }
}
