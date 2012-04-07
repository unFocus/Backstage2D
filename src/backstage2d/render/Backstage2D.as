package backstage2d.render 
{
	import backstage2d.display.*;
	import backstage2d.render.stage3d.*;
	
	import flash.display.Stage3D;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public class Backstage2D 
	{
		public var context:Context;
		
		protected var _width:Number;
		public function set width( width:Number ):void {
			_width = width;
			context.configure(_width, _height, _quality );
		}
		public function get width():Number {
			return _width;
		}
		
		protected var _height:Number;
		public function set height( height:Number ):void {
			_height = height;
			context.configure(_width, _height, _quality );
		}
		
		public function setSize( width:Number, height:Number ):void {
			_width = width;
			_height = height;
			context.configure( width, height );
		}
		
		protected var _quality:Number = AntiAliasQuality.NONE;
		public function set quality( quality:Number ):void {
			_quality = quality;
			context.configure(_width, _height, _quality );
		}
		public function get quality():Number {
			return _quality;
		}
		
		internal const batches:Vector.<Batch> = new <Batch>[];
		
		/**
		 * The constructor for the Renderer.
		 * @param	width
		 * @param	height
		 * @param	stage3D
		 */
		public function Backstage2D( width:Number, height:Number, stage3D:Stage3D ) 
		{
			_width = width;
			_height = height;
			
			// :NOTE: This fires whenever the context3D is created, for example,
			// after the lock screen is activated on Windows devices.
			context = new Context( stage3D );
			context.created.add( onContextCreated );
		}
		
		/**
		 * 
		 * @param	event
		 */
		protected function onContextCreated( context:Context ):void
		{
			this.context = context;
			context.configure(_width, _height, _quality);
			for each ( var batch:Batch in batches ) {
				batch.activate( context );
			}
		}
		
		public function addLayer( layer:Layer ):Batch
		{
			var batch:Batch = new Batch( layer );
			batches.push( batch );
			return batch;
		}
		
		// :TODO: Rename this to make or build or create?
		public function start():void {
			for each ( var batch:Batch in batches ) {
				batch.build();
			}
		}
		
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
		
		public function clear( color:uint ):void
		{
			context.context3D.clear();
		}
	}
}
