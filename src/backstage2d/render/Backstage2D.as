package backstage2d.render 
{
	import backstage2d.display.*;
	import backstage2d.render.stage3d.*;
	
	import flash.display.Stage3D;
	
	/**
	 * Backstage2D is the main entry point for setting up and managing the back stage.
	 * @author Kevin Newman
	 */
	public class Backstage2D 
	{
		/**
		 * The context which holds the GPU resource and configuration.
		 */
		public var context:Context;
		
		internal const batches:Vector.<Batch> = new <Batch>[];
		
		/**
		 * The constructor for the Backstage2D display list augmentation engine.
		 * @param	width
		 * @param	height
		 * @param	stage3D
		 */
		public function Backstage2D( stage3D:Stage3D ) 
		{
			// :NOTE: This fires whenever the context3D is created, for example,
			// after the lock screen is activated on Windows devices.
			context = new Context( stage3D );
			context.created.add( onContextCreated );
		}
		
		/**
		 * Handles device initialization and recovery.
		 * @param	event
		 */
		protected function onContextCreated( context:Context ):void
		{
			this.context = context;
		}
		
		/**
		 * Add a layer to the augmentation engine. Each Layer is rendered as a Batch.
		 * A Batch can be configured and tuned.
		 * @param	layer The layer to be rendered.
		 * @return	The batch created for the layer.
		 */
		public function addLayer( layer:Layer ):Batch
		{
			var batch:Batch = new Batch( layer );
			batches.push( batch );
			return batch;
		}
		
		/**
		 * Builds all the layers in the backstage.
		 */
		public function build():void
		{
			for each ( var batch:Batch in batches ) {
				batch.build();
			}
		}
		
		/**
		 * Starts the augmentation engine. This will build any unbuilt layer/batches,
		 * and start the renderer.
		 */
		public function start():void
		{
			for each ( var batch:Batch in batches ) {
				batch.activate( context );
			}
		}
		
		/**
		 * The main render hook. This must be called explicitly in your main RENDER or ENTER_FRAME loop. Renders all batches/layers.
		 */
		public function renderFrame():void
		{
			if ( !context.verify() ) {
				return;
			}
			
			for ( var i:int = 0, n:uint = batches.length; n > i; ++i )
			{
				batches[ i ].draw();
			}
			
			context.context3D.present();
		}
		
		/**
		 * Clears the stage, and fills with an ARGB color.
		 * @param	color
		 */
		public function clear( color:uint = 0 ):void
		{
			context.context3D.clear(
				( color >> 16 ) & 0xFF,	// r
				( color >> 8) & 0xFF,	// g
				color & 0xFF,			// b
				color >> 24				// a
			);
		}
		
	}
}
