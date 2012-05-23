package
{
	import backstage2d.display.classic.BSSprite;
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
	final public class MolePeopleDL extends Sprite
	{
		private var backstage:Backstage2D;
		
		private var image:BSSprite;
		private var image2:BSSprite;
		private var layer:Layer;
		private var image3:BSSprite;
		
		public function MolePeopleDL()
		{
			if (stage) init();
			else addEventListener(Event.ADDED_TO_STAGE, init);
		}
		private function init( event:Event = null ):void
		{
			var sprite:Sprite = new MoleShadow;
			
			image = new BSSprite( sprite );
			image.name = "sprite1 tl";
			image2 = new BSSprite( sprite );
			image2.name = "sprite2 c ";
			image3 = new BSSprite( sprite );
			image3.name = "sprite3 tr ";
			
			layer = new Layer();
			layer.add( image );
			layer.add( image2 );
			layer.add( image3 );
			image2.opacity = 0.5;
			image2.registration.x = image2.width / 2;
			image2.registration.y = image2.height / 2;
			image3.registration.x = image3.width;
			image3.registration.y = image3.height;
			image3.opacity = .25;
			
			backstage = new Backstage2D( stage.stageWidth, stage.stageHeight, stage.stage3Ds[0] );
			
			backstage.context.created.add( onContextCreated );
			backstage.context.failed.add( onContextFail );
			backstage.context.lost.add( onContextLost );
			
			// setup
			var batch:Batch = backstage.addLayer( layer );
			
			backstage.start();
			//addChild( new Bitmap( batch.atlas.ss, "auto", true ) );
		}
		
		private function onResize( event:Event ):void {
			backstage.setSize( stage.stageWidth, stage.stageHeight );
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
			
			backstage.quality = AntiAliasQuality.HIGH;
			addEventListener( Event.ENTER_FRAME, onEnterFrame );
		}
		
		private function gameTick():void
		{
			var timeSec:Number = getTimer() / 500;
			
			image3.x = image2.x = image.x = stage.stageWidth / 2;
			image3.y = image2.y = image.y = stage.stageHeight / 2;
			//image.registration.x = 0;
			//image.registration.y = 0;
			image.rotation += 0.01;
			image2.rotation -= 0.02;
			
			image.scaleX   = 2 + Math.sin(timeSec);
			image.scaleY   = 2 + Math.sin(timeSec);
		}
		
		private function onEnterFrame(event:Event):void
		{
			gameTick();
			backstage.clear(0xFFFFFF);
			backstage.renderFrame();
		}
		
	}
}
