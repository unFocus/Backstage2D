package
{
	import backstage2d.display.Image;
	import backstage2d.display.Layer;
	
	import backstage2d.render.Backstage2D;
	import backstage2d.render.stage3d.AntiAliasQuality;
	import backstage2d.render.stage3d.Batch;
	import backstage2d.render.stage3d.Context;
	
	import flash.display.BitmapData;
	import flash.display.Sprite;
	import flash.events.Event;
	import flash.text.TextField;
	import flash.utils.getTimer;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	final public class MolePeopleTest extends Sprite
	{
		private var backstage:Backstage2D;
		
		private var image:Image;
		private var image2:Image;
		private var layer:Layer;
		
		public function MolePeopleTest()
		{
			if (stage) init();
			else addEventListener(Event.ADDED_TO_STAGE, init);
		}
		private function init( event:Event = null ):void
		{
			var bmd:BitmapData = new MolePeopleBitmap().bitmapData;
			image = new Image( bmd );
			image2 = new Image( bmd );
			
			layer = new Layer();
			layer.add( image );
			layer.add( image2 );
			image2.opacity = 0.5;
			
			backstage = new Backstage2D( stage.stage3Ds[0] );
			backstage.context.configure( stage.stageWidth, stage.stageHeight, AntiAliasQuality.HIGH );
			backstage.context.created.add( onContextCreated );
			backstage.context.failed.add( onContextFail );
			backstage.context.lost.add( onContextLost );
			
			// setup
			var batch:Batch = backstage.addLayer( layer );
			
			backstage.start();
			//addChild( new Bitmap( batch.atlas.ss, "auto", true ) );
		}
		
		private function onResize( event:Event ):void {
			backstage.context.configure( stage.stageWidth, stage.stageHeight, AntiAliasQuality.HIGH );
		}
		
		private function onContextLost():void 
		{
			trace('Context3D lost');
			removeEventListener( Event.ENTER_FRAME, onEnterFrame );
		}
		
		private function onContextFail( event:Event ):void 
		{
			trace('Context3D unavailable');
			var textFld:TextField = new TextField();
			addChild( textFld );
			textFld.text = "Context3D unavailable";
		}
		
		private function onContextCreated( context:Context ):void
		{
			trace("Context3D created");
			
			addEventListener( Event.ENTER_FRAME, onEnterFrame );
		}
		
		private function gameTick():void
		{
			var timeSec:Number = getTimer() / 500;
			
			image2.x = image.x = stage.stageWidth / 2;
			image2.y = image.y = stage.stageHeight / 2;
			
			image.rotation += 0.01;
			image2.rotation -= 0.02;
			
			image.scaleX   = 2 + Math.sin(timeSec);
			image.scaleY   = 2 + Math.sin(timeSec);
		}
		
		private function onEnterFrame(event:Event):void
		{
			gameTick();
			backstage.clear();
			backstage.renderFrame();
		}
		
	}
}
